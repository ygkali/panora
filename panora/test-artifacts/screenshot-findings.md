# Screenshot findings

## 02-gui-history.png

Panora GTK4/libadwaita popup Xvfb üzerinde açıldı. Başlık çubuğunda `Ayarlar` ve `Özel` kontrolleri, ortada odaklanmış arama alanı, listede iki text geçmiş kaydı ve altta klavye ipucu görünüyor. Pencere 1280x800 sanal ekranın sol üstünde yaklaşık 720x560 boyutunda açılmış; gerçek window manager olmadığı için ekranın geri kalanı siyah kalmış.

## 03-gui-search.png

Arama alanına `Merhaba` yazıldığında liste tek kayda filtrelendi: `Merhaba Panora - GUI testi`. Bu görüntü GUI → Unix socket → daemon → SQLite/FTS5 arama yolunun görsel kanıtıdır. Arama alanı odaklanmış ve Türkçe kullanıcı metinleri doğru görüntülenmiştir.

## 04-gui-settings.png

Ayarlar penceresi başarılı şekilde açıldı. Görünen kontroller: sistem dili seçimi, maksimum kayıt sayısı (1000), saklama süresi (30 gün), MIME başına boyut (10 MiB), özel mod anahtarı, anında yapıştır anahtarı, hariç tutulan uygulamalar metin alanı ve Kaydet/Kapat düğmeleri. Pencere popup'ın üzerinde konumlanıyor; Xvfb'de window manager olmadığı için arka popup kısmen görünür ve ekranın geri kalanı siyah.

İlk denemede ayarlar görüntüsü siyah çıktı; bunun nedeni Escape ile popup'ın gizlenmesi ve xdotool koordinat sözdiziminin hatalı olmasıydı. Senaryo düzeltilip aynı ekran tekrar alındı; bu dosyadaki 04 görüntüsü geçerli başarılı görüntüdür.
