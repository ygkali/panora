# Panora protocol and format matrix

| Alan | Destek | Uygulama notu |
|---|---|---|
| X11 CLIPBOARD | Evet | `xclip` alt süreci (TARGETS/TIMESTAMP), 180 ms poll; kaynak uygulama adı x11rb ile `_NET_ACTIVE_WINDOW`→`WM_CLASS` |
| X11 PRIMARY | Model/trait hazır | v1 config default kapalı; backend capability opt-in |
| Wayland regular clipboard | Evet | `wl-paste`/`wl-copy` alt süreci (wl-clipboard paketi), 180 ms poll |
| Wayland primary | Compositor destekliyorsa | `ClipboardType::Primary`; capability error safe şekilde yüzeye çıkarılır |
| GNOME Wayland | Evet, bridge ile | Shell extension `St.Clipboard` → session D-Bus → Rust privacy gate |
| KDE/wlroots | Backend hedefi | ext/wlr data-control protokol desteğine bağlı; GNOME extension gerekmez |
| `text/plain`, `UTF8_STRING` | Tam | Preview + FTS5 arama + recall |
| `text/html`, `text/rtf` | Model/selection hazır | Text fallback; rich payload backend'e göre değişebilir |
| `text/uri-list` | Model hazır | URI/file-list preview ve MIME payload saklama |
| `x-special/gnome-copied-files` | Model/policy hazır | GNOME file-list payload; bridge v1 text odaklı |
| `image/png`, JPEG, WebP, BMP, TIFF, SVG | Model/policy hazır | X11 image offer; Wayland raw MIME offer; GUI lazy preview sonraki UI iterasyonu |
| Renk kodları | Model sınıflandırması | `#rrggbb`, `rgb()` metinleri için `ContentKind::Color` |
| Parola bayrakları | Güvenlik kapısı | `x-kde-passwordManagerHint`, ConcealedType varyantları payload okunmadan reddedilir |
| Hariç tutulan uygulamalar | X11 ve GNOME'da evet, Wayland'de **hayır** | Filtre `source_app`'e dayanır. X11'de odaklı pencerenin `WM_CLASS`'ı, GNOME'da extension'ın verdiği app id kullanılır. Düz Wayland'de data-control protokolü istemci kimliği sunmadığı için bu liste **devreye girmez**; MIME bayrağı kapısı çalışmaya devam eder |
| IPC | Unix socket | `0600`, JSON-lines, `io.panora.Pano1` protokol sabitleri |
| GNOME bridge D-Bus | Evet | `io.panora.GnomeBridge1`, bounded channel, final privacy check Rust'ta |
| Core D-Bus UI API | Sabitler hazır | `org.panora.Pano1` uyumluluk adı; v1 GUI hızlı yol olarak Unix socket kullanır |
| Sync/network | Hayır (v1) | `panora-sync::Transport::Disabled`; uygulama ağ açmaz |

## Gerçek oturum testleri

X11 için Xvfb altında clipboard provider ve `xclip` ile capture–recall; Wayland için Sway headless veya gerçek GNOME 46–51; GNOME extension için `gnome-extensions` ve Shell restart ile manuel doğrulama yapılmalıdır. Sandbox'ta bu compositor testleri mevcut değildir ve release checklist'inde açıkça işaretlenmiştir.
