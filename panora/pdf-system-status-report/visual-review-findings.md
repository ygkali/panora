# PDF görsel inceleme bulguları

Typst strict derleme başarılı oldu; PDF doğrulayıcı `PASS pass=6 warn=0 fail=0 unknown=0` verdi. Standart görsel inceleme 9 sayfanın 6 temsilî sayfasını üretti.

Contact sheet incelemesinde başlık, tablolar, Türkçe karakterler, sayfa numaraları ve referans bölümü doğru göründü. Sayfa 6'da kaynak kullanım tablosu ve sistem/masaüstü uyumluluk matrisi sayfa genişliğine sığıyor; hücrelerde kontrollü satır kırılması var ve tablo dışına taşma görülmedi. Kaynak kullanım tablosunun daemon satırı önceki sayfada başladığı için sayfa 6'da yalnızca GUI satırının görünmesi normal sayfa akışıdır. Uyumluluk tablosu `Ubuntu + X11`, `Debian + X11`, `GNOME X11`, `GNOME Wayland` ve `Sway/KDE Wayland` satırlarını okunabilir biçimde taşıyor.

PDF teslim edilebilir durumda. Görsel olarak dikkat edilmesi gereken tek nokta bazı geniş tablolarda dar sütunların satırları uzatmasıdır; içerik kaybı, kesilmiş hücre veya eksik Türkçe glyph gözlenmedi.
