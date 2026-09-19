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
| GNOME Wayland ≤ 47 (Zorin OS 18 / GNOME 46 dahil) | Evet, bridge ile | Shell eklentisi `MetaSelection::owner-changed` → `St.Clipboard.get_mimetypes/get_content` → session D-Bus `io.github.ygkali.Panora.GnomeBridge1.PushMany(as mimes, a(say) payloads, s source_app)` → Rust privacy gate. Görüntü varsa yalnızca en iyi görüntü (`image/png` > `jpeg` > `webp`); yoksa en iyi düz metin + `text/html` + `text/uri-list` + `x-special/gnome-copied-files`. Eski daemon `UnknownMethod` dönerse tek payload'lı `Push(assays)` ile ilk payload gönderilir. Geri çağırma `io.github.ygkali.Panora.GnomeShell1.SetClipboard` (`St.Clipboard.set_content` tek MIME sunar); `SetClipboard` sonrası 1,5 s kendi yankısı yakalanmaz |
| GNOME oturum modu | Evet | `metadata.json` `session-modes: ["user", "zorin"]`; Zorin `GNOME_SHELL_SESSION_MODE=zorin` ile çalışır, Shell yalnızca `currentMode` veya `parentMode` listede olan eklentileri yükler |
| Super+V çakışması | Eklenti çözer | GNOME varsayılanı `org.gnome.shell.keybindings toggle-message-tray = ['<Super>v','<Super>m']`; Mutter aynı kombinasyon için tek binding tutar (hash tablosu, sıra belirsiz). `enable()` çakışan girdiyi kaldırır (Super+M kalır), orijinali `restore-message-tray` anahtarına yazar, `disable()` geri yükler. Zorin'in kendi Super+V kısayolu yok |
| Popup odağı | Evet | `Activate` çağrısına Shell'in `create_app_launch_context().get_startup_notify_id()` ile ürettiği `activation-token`/`desktop-startup-id` eklenir (xdg-activation / X11 startup notification); zaman aşımı 25 s, yalnızca `ServiceUnknown`/`NameHasNoOwner` durumunda ikili elle başlatılır |
| KDE/wlroots | Evet | data-control; GNOME extension gerekmez |
| Wayland anında yapıştır | Kısmi | GNOME: eklenti `Paste()` → Clutter sanal klavye `notify_key` evdev kodları (KEY_LEFTCTRL=29, KEY_V=47; düzenden bağımsız, Wayland ve XWayland istemcileri için aynı yol). Odak hâlâ Panora popup'ındaysa 700 ms'ye kadar 25 ms aralıkla beklenir, sonra `…Error.Focus` döner; diğerleri: `wtype` veya `ydotool` kuruluysa |
| `text/plain`, `UTF8_STRING` | Tam | Preview + FTS5 önek arama + recall |
| `text/html`, `text/rtf` | Tam | Düz metinle birlikte saklanır ve sunulur; yalnızca HTML varsa etiketler ayıklanarak önizleme üretilir |
| `text/uri-list`, `x-special/gnome-copied-files` | Tam | Dosya adları önizlemede; dosya yöneticisine geri çağırma aynı MIME'larla |
| `image/png`, JPEG, WebP, BMP, TIFF, GIF, SVG, AVIF, HEIC | Tam | Kart küçük resmi ve tam boy ayrıntı görünümü; `panora-cli preview --out` ile dışa aktarma |
| Renk kodları | Evet | `#rrggbb`, `rgb()` → `ContentKind::Color`, renk örneği çizilir |
| Parola bayrakları | Güvenlik kapısı | `x-kde-passwordManagerHint`, `ConcealedType`, `Clipboard Viewer Ignore` varyantları payload okunmadan reddedilir |
| Hariç tutulan uygulamalar | X11 ve GNOME'da evet, düz Wayland'de **hayır** | Filtre `source_app`'e dayanır. Düz Wayland data-control protokolü istemci kimliği sunmadığı için bu liste orada devreye giremez; MIME bayrağı kapısı çalışmaya devam eder. Backend bunu `Capabilities::source_app` ile bildirir, `Status` üzerinden `panora-cli status` (`source_app=`) ve ayarlar penceresi bunu gösterir — liste boşuna güvenilmesin diye |
| GNOME köprüsü çağıran kimliği | Doğrulanır | `io.github.ygkali.Panora.GnomeBridge1` oturum veriyolundadır, yani her kullanıcı süreci erişebilir. `Push`/`PushMany` gönderenin unique adını `org.gnome.Shell` sahibiyle karşılaştırır (eklenti gnome-shell'in paylaşılan oturum bağlantısını kullanır); eşleşmezse `AccessDenied`. Kötü amaçlı bir Shell eklentisi ADR 0003 tehdit modelinin dışındadır |
| IPC | Unix socket | `0600`, JSON-lines, protokol sürümü 2, base64 payload'lar, 64 KiB istek / 64 MiB yanıt sınırı, peer UID kontrolü |
| GUI etkinleştirme | D-Bus | `io.github.ygkali.Panora` `org.freedesktop.Application.Activate`; ikinci etkinleştirme popup'ı kapatır |
| Sync/network | Hayır | `SyncProvider` trait + `NoopSync`; uygulama ağ açmaz |

## Gerçek oturum testleri

X11 backend'i `crates/panod/tests/x11_integration.rs` ile Xvfb altında test edilir (offer→read, INCR, XFIXES watch, kendi sahipliğini yok sayma). Wayland backend'i için Sway headless veya gerçek GNOME 48+/KDE oturumu, GNOME bridge için `gnome-extensions` ile manuel doğrulama gerekir; bunlar release kontrol listesinde manuel adım olarak işaretlidir.
