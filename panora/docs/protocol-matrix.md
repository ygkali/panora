# Panora protocol and format matrix

| Alan | Destek | Uygulama notu |
|---|---|---|
| X11 CLIPBOARD | Evet | XFIXES `SelectionNotify` olayları (poll yok); TARGETS `ConvertSelection` ile payload okunmadan alınır; INCR okuma/yazma; kaynak uygulama adı `_NET_ACTIVE_WINDOW`→`WM_CLASS` |
| X11 PRIMARY | Evet (opt-in) | `history.record_primary = true`; aynı XFIXES yolu |
| X11 geri çağırma | Evet | panod selection owner olur; TARGETS/TIMESTAMP + tüm biçimler + metin takma adları (`UTF8_STRING`, `STRING`, `TEXT`); 256 KiB üstü INCR |
| X11 kalıcılık | Evet | `SelectionWindowDestroy`/`SelectionClientClose` → yalnızca son değişiklikten kaydedilen içerik yeniden sunulur; bilinçli temizleme (owner=None) yok sayılır |
| Wayland kalıcılık | Bileşim yöneticisine bırakılır | `selection(null)` bilinçli temizlemeyle ayırt edilemez; Mutter/KWin içeriği kendileri korur |
| X11 anında yapıştır | Evet | XTEST `Ctrl+V` |
| Wayland regular clipboard | Evet | `ext-data-control-v1` (tercih) veya `zwlr-data-control-v1` (wayland-client, alt süreç yok); `selection` olayı MIME listesiyle gelir, payload `receive` ile pipe üzerinden yalnızca izin sonrası okunur |
| Wayland primary | Evet (opt-in) | `ext` her sürümde, `wlr` v2+ |
| Wayland geri çağırma | Evet | data source tüm biçimler + metin takma adları; `send` istekleri arka plan iş parçacığında servis edilir |
| GNOME Wayland 48+ | Evet, yerel | Mutter `ext-data-control-v1`; Shell eklentisi yalnızca Super+V ve yapıştırma için, bridge push'ları yok sayılır |
| GNOME Wayland ≤ 47 | Evet, bridge ile | Shell eklentisi `St.Clipboard` → session D-Bus `io.panora.GnomeBridge1.Push` → Rust privacy gate; geri çağırma `io.panora.GnomeShell1.SetClipboard` |
| KDE/wlroots | Evet | data-control; GNOME extension gerekmez |
| Wayland anında yapıştır | Kısmi | GNOME: eklenti (Clutter virtual keyboard); diğerleri: `wtype` veya `ydotool` kuruluysa |
| `text/plain`, `UTF8_STRING` | Tam | Preview + FTS5 önek arama + recall |
| `text/html`, `text/rtf` | Tam | Düz metinle birlikte saklanır ve sunulur; yalnızca HTML varsa etiketler ayıklanarak önizleme üretilir |
| `text/uri-list`, `x-special/gnome-copied-files` | Tam | Dosya adları önizlemede; dosya yöneticisine geri çağırma aynı MIME'larla |
| `image/png`, JPEG, WebP, BMP, TIFF, GIF, SVG, AVIF, HEIC | Tam | Kart küçük resmi ve tam boy ayrıntı görünümü; `panora-cli preview --out` ile dışa aktarma |
| Renk kodları | Evet | `#rrggbb`, `rgb()` → `ContentKind::Color`, renk örneği çizilir |
| Parola bayrakları | Güvenlik kapısı | `x-kde-passwordManagerHint`, `ConcealedType`, `Clipboard Viewer Ignore` varyantları payload okunmadan reddedilir |
| Hariç tutulan uygulamalar | X11 ve GNOME'da evet, düz Wayland'de **hayır** | Filtre `source_app`'e dayanır. Düz Wayland data-control protokolü istemci kimliği sunmadığı için bu liste orada devreye giremez; MIME bayrağı kapısı çalışmaya devam eder |
| IPC | Unix socket | `0600`, JSON-lines, protokol sürümü 2, base64 payload'lar, 64 KiB istek / 64 MiB yanıt sınırı, peer UID kontrolü |
| GUI etkinleştirme | D-Bus | `io.panora.Panora` `org.freedesktop.Application.Activate`; ikinci etkinleştirme popup'ı kapatır |
| Sync/network | Hayır | `SyncProvider` trait + `NoopSync`; uygulama ağ açmaz |

## Gerçek oturum testleri

X11 backend'i `crates/panod/tests/x11_integration.rs` ile Xvfb altında test edilir (offer→read, INCR, XFIXES watch, kendi sahipliğini yok sayma). Wayland backend'i için Sway headless veya gerçek GNOME 48+/KDE oturumu, GNOME bridge için `gnome-extensions` ile manuel doğrulama gerekir; bunlar release kontrol listesinde manuel adım olarak işaretlidir.
