# Panora UI ve format runtime görsel bulguları

- Release `panora-gui`, Xvfb :131 üzerinde 1280x800 ekranda açıldı.
- İki sütunlu kart düzeni, üst arama alanı, Özel/Temizle kontrolleri ve kart başına `Koy`, `Pin`, `×` aksiyonları görünür.
- Metin kartı `METİN`, HTML kartı `RICH`, URI kartı `LINK`, fotoğraf kartı `FOTO` rozetiyle göründü.
- PNG fotoğraf payload'ı şifreli storage'dan alındı ve yaklaşık 320x180 hedefli thumbnail olarak kart içinde gösterildi.
- GUI ekran görüntüsü: `release-ui-runtime-gui.png`.
- Sağ kalan siyah alan Xvfb'de pencerenin ekranın sol üstüne yerleşmesinden kaynaklanan masaüstü alanıdır; GUI penceresi içinde siyah geçiş/kayıp görünmedi.
