# Video görsel test notları

Önceki GUI ekran görüntülerinde Panora penceresi 1280x800 sanal X11 ekranının sol üst bölümünde açılıyor. Geçmiş ekranında üstte `Ayarlar`, `Özel`, pencere düğmeleri ve odaklanmış `Pano geçmişinde ara...` alanı var. Liste satırları içerik türü ve kısa preview ile gösteriliyor; alt yardım çubuğu yön tuşlarıyla seçim, Enter ile panoya kopyalama, Ctrl+F ile arama ve Escape ile kapatma akışını açıklıyor.

Ayarlar ekranında arayüz dili, maksimum kayıt, saklama süresi, MIME başına boyut, özel mod, anında yapıştır ve hariç tutulan uygulamalar kontrolleri bulunuyor. Alt bölümde `Kapat` ve `Kaydet` düğmeleri var. Video senaryosu bu kontrolleri fare koordinatlarına bağlı kalmadan klavye/tab akışı ve görünen düğme koordinatlarıyla kullanacak.

## İlk MP4 kalite kontrolü

İlk 90,7 saniyelik MP4 teknik olarak geçerli H.264/yuv420p üretildi; ancak görsel kare kontrolünde Panora popup'ının Escape/Enter akışlarından sonra kapanması nedeniyle kaydın büyük bölümünde siyah sanal masaüstü göründü. 30. saniye karesinde yalnızca çoklu format başlık terminali, 75. saniye karesinde ise siyah masaüstü kaldı. Bu nedenle kayıt başarısız sayıldı ve popup'ı her kapanan işlemden sonra yeniden açan, ayrıca terminal penceresini daha görünür tutan ikinci bir senaryo hazırlanmalıdır.

## İkinci MP4 kalite kontrolü

Düzeltilmiş senaryonun MP4'si 126,6 saniye, 1280×800, H.264/yuv420p ve yaklaşık 691 KiB boyutunda üretildi. 15. saniye karesinde Panora GUI ve iki metin kaydı net biçimde görünür durumdadır. Ancak 30. saniye karesi yeniden tamamen siyah sanal masaüstünü göstermektedir; dolayısıyla GUI'yi yeniden başlatmak tek başına yeterli olmamıştır. Contact sheette bazı kareler görünür olsa da kayıt hâlâ kesintili ve teslim kalitesinde değildir. Bir sonraki denemede pencere yönetimi/odak davranışını sadeleştirip GUI'yi tam ekran veya sabit bölünmüş düzende sürekli görünür tutmak gerekir.

## Zaman çizelgesi kontrolü

5 saniyelik aralıklı contact sheet, ikinci kayıtta GUI'nin yaklaşık 5–20, 30–45, 55–85 ve 95–120. saniye aralıklarında görünür olduğunu; siyah karelerin ise GUI yeniden başlatılırken oluşan kısa geçiş boşluklarına denk geldiğini gösterdi. 105. saniye spot karesi private mode öncesi kaydın GUI içinde net göründüğünü doğruladı. Bu nedenle siyah ekran ilk MP4'teki gibi sürekli bir arıza değil, v2 senaryosundaki stop/start geçişlerinden kaynaklanan kısa aralıklı görüntü kesintileridir.

## Ayarlar ve teslim kararı

95. saniye spot karesi ayarlar penceresini okunabilir biçimde gösteriyor: arayüz dili, maksimum kayıt, saklama süresi, MIME başına boyut, özel mod, anında yapıştır, hariç tutulan uygulamalar ve `Kapat`/`Kaydet` düğmeleri kadrajda. Görsel denetim sonucunda v2 MP4, kısa geçiş siyah kareleri dışında kullanıcı tarafından izlenebilir ve özellik turunu kanıtlayan yeterli kaliteye ulaştı; teslim için ana video, contact sheet, teknik medya bilgisi, test raporu ve destekleyici kareler birlikte hazırlanacaktır.
