# Panora 1.2.0 — Lokal Kurulum Kiti

## Zorin OS 18 / Ubuntu 24.04

Panora, Zorin OS 18 (Ubuntu 24.04 tabanı, GNOME 46), Ubuntu 24.04+ ve Debian 13+ üzerinde test edilmek üzere hazırlandı; Zorin OS 17 (Ubuntu 22.04) libadwaita 1.1 ile geldiğinden desteklenmez ve `KUR.sh` bunu açıkça söyleyip durur. Zorin'in varsayılan Wayland oturumunda pano yakalama GNOME eklentisi üzerinden yapılır; kurulumdan sonra **oturumu kapatıp açın**, ardından `gnome-extensions enable panora@panora-clipboard.org` (KUR.sh zaten dener) ve `panora-doctor` ile durumu kontrol edin. Eklenti etkinleşince Super+V, GNOME'un bildirim listesi kısayolundan alınıp Panora'ya verilir (eklenti kapatılınca geri döner).

## En hızlı kurulum

Bu klasörde terminalde şu komutları çalıştırın:

```bash
chmod +x KUR.sh TEST.sh KALDIR.sh
./KUR.sh
```

`KUR.sh` Debian/Ubuntu bağımlılıklarını kurar, Rust araç zinciri yoksa rustup ile kullanıcı dizinine indirir, Panora'yı kaynaktan derleyip `.deb` paketini üretir, yükler ve kullanıcı daemon servisini (`panod.service`) başlatır. Sudo parolanız istenebilir. İlk derleme birkaç dakika sürer; hazır bir `dist/panora_*.deb` varsa ve makinenizle uyumluysa derleme atlanır.

Kurulumdan sonra test:

```bash
panora-doctor          # ortam/servis/eklenti teşhisi (OK / UYARI / HATA)
./TEST.sh              # teşhis + hızlı pano testi + GUI
panora/scripts/e2e-test.sh --safe   # tüm özellikler için PASS/FAIL raporu
```

Test daemon durumunu, gerçek bir pano kopyasını (X11'de `xclip`, Wayland'de `wl-copy` veya GNOME eklentisi varsa onun üzerinden), listeleme/FTS5 aramayı ve geri çağırmayı doğrular, ardından GUI'yi açar. Yalnızca terminal testi için:

```bash
PANORA_NO_GUI=1 ./TEST.sh
```

GUI'yi elle açmak / kapatmak için `panora` (ikinci çağrı açık pencereyi kapatır). GNOME'da oturumu yeniden açtıktan sonra Super+V:

```bash
gnome-extensions enable panora@panora-clipboard.org
```

## Görsel ve biçimli içerik testi

Bir görsel kopyalayın (ekran görüntüsü, tarayıcıdan "Resmi kopyala") ve `panora` içinde **GÖRSEL** kartını ve küçük resmi görün; `Space` tam boy önizlemeyi açar. Tarayıcıdan biçimli metin kopyaladığınızda kayıt **BİÇİMLİ** olarak işaretlenir ve geri çağırma hem HTML'i hem düz metni sunar. `panora-cli preview <id> --mime image/png --out foto.png` ile payload'ı dosyaya yazabilirsiniz.

## Kaynak koddan derleme

```bash
cd panora
sudo apt install -y build-essential pkg-config libgtk-4-dev libadwaita-1-dev
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --workspace --release
```

Rust 1.85 veya daha yeni stable toolchain gereklidir. Sistem uyumluluğu, servis günlükleri ve sorun giderme için `panora/INSTALL-local.md` dosyasını okuyun.

## Kaldırma

```bash
./KALDIR.sh
```

Kaldırma script'i programı ve servisi kaldırır, fakat şifreli clipboard geçmişini varsayılan olarak silmez. Verileri de silmek isterseniz `~/.local/share/panora` ve `~/.config/panora` yollarını ayrıca ve bilinçli olarak kaldırın.

## İçerik

`panora/` klasörü Rust kaynaklarını (daemon, GTK4/libadwaita GUI, CLI), yerel X11/Wayland backend'lerini, GNOME Shell eklentisini, paketleme script'lerini, testleri ve belgeleri içerir. Hazır paket kitle birlikte gelmez; `KUR.sh` (veya `panora/packaging/build-deb.sh`) onu bu makinede üretir ve SHA-256 özetini yazdırır.
