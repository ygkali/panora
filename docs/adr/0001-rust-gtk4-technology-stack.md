# ADR 0001: Rust + GTK4/libadwaita Teknoloji Yığını

- **Durum:** Kabul edildi
- **Tarih:** 2026-08-18

## Bağlam

Panora; Debian tabanlı sistemlerde GNOME öncelikli, düşük RAM tüketimli, güvenli bir pano yöneticisidir. Daemon + GUI + CLI tek dilde yazılmak istenmektedir. Adaylar: Rust+GTK4, C++/Qt6, Go+Fyne, Python+GTK.

## Karar

**Rust** (daemon, çekirdek, CLI) + **GTK4/libadwaita** (GUI) + **gtk4-layer-shell** (destekleyen ortamlarda overlay popup).

## Gerekçe

1. **Bellek güvenliği ve düşük RAM:** Pano yöneticisi hassas veri işler; Rust'ın sahiplik modeli bellek hatalarını derleme zamanında engeller. Boşta <30 MB hedefi Rust ile gerçekçidir (Maccy referansı 14–22 MB).
2. **Wayland pano ekosistemi:** `wl-clipboard-rs` crate'i `ext-data-control-v1` ve `wlr-data-control-unstable-v1` protokollerini pencere açmadan soyutlar — pano yöneticisi için idealdir. X11 tarafında `x11rb` saf Rust'tır ve XFixes/INCR kontrolü verir.
3. **GNOME'da yerel görünüm:** GTK4 + libadwaita, birincil hedef masaüstü olan GNOME'da yerel görünür; KDE'de kabul edilebilir durur. `gtk4-layer-shell` ile wlroots/KDE'de gerçek overlay popup mümkündür.
4. **Kripto ekosistemi:** XChaCha20-Poly1305, Argon2id, BLAKE3 için olgun, denetlenmiş crate'ler mevcuttur.
5. **Tek dil:** Daemon, GUI ve CLI aynı dilde; tip paylaşımı IPC sözleşmelerini güvenli kılar.

## Alternatifler ve Red Gerekçeleri

| Aday | Red gerekçesi |
|---|---|
| C++/Qt6 | Olgun (CopyQ kanıtı) ama GNOME'da yabancı görünüm; bellek güvenliği Rust kadar güçlü değil |
| Go + Fyne | Wayland layer-shell desteği yok; GUI yerel hissi zayıf |
| Python + GTK | Daemon için performans ve dağıtım zayıf |
| Tauri | WebView runtime'ı pano popup'ı için ağır; soğuk açılış <100 ms hedefini riske atar |

## Sonuçlar

- **Olumlu:** Güvenlik, performans, GNOME entegrasyonu, tek dilde bakım kolaylığı.
- **Olumsuz:** GTK4 Rust binding'leri (`gtk4-rs`) öğrenme eğrisi; `gtk4-layer-shell` GTK sürümleriyle uyumluluk takibi gerektirir (sürüm sabitleme + CI ile azaltılır).
- **Nötr:** GNOME'da layer-shell yoktur; popup GNOME'da ortalanmış modal pencere olarak açılır (kabul edilmiş düşüş).
