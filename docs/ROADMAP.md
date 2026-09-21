# Panora — Yayın Hazırlığı ve Yayın Sonrası Geliştirme Yol Haritası

**Tarih:** 2026-09-19
**Sürüm bağlamı:** 1.2.0 + `CHANGELOG.md` "Yayınlanmadı" bölümündeki değişiklikler (commit `3dde166`)
**Depo:** https://github.com/ygkali/panora (public, CI yeşil: run 35380401689)
**Amaç:** Projeyi herkese açık ilk sürüme hazırlamak ve sonrasında hangi özelliklerin, hangi sırayla ekleneceğini tek bir yerden yönetmek. Bu belge geliştirme boyunca güncellenecek "ana plan"dır; her madde bir GitHub issue'ya dönüştürülebilecek kadar ayrıntılıdır.

---

## 0. Belgeyi okuma kılavuzu

**Madde kimlikleri**

| Önek | Alan |
|---|---|
| `R-` | Yayın öncesi zorunlu iş (release blocker) |
| `B-` | Analiz sırasında tespit edilen hata / tutarsızlık |
| `CAP-` | Pano yakalama ve backend'ler (`crates/panod/src/backend/*`, `daemon.rs`) |
| `UI-` | Popup ve ayarlar (`crates/panora-gui`) |
| `CLI-` | Komut satırı istemcisi (`crates/panora-cli`) |
| `SEC-` | Gizlilik, güvenlik, tedarik zinciri |
| `STO-` | Depolama, IPC, performans (`crates/panora-core/src/storage`, `ipc.rs`) |
| `INT-` | Masaüstü entegrasyonu (GNOME eklentisi, KDE, wlroots, Xfce…) |
| `PKG-` | Paketleme ve dağıtım |
| `I18N-` | Yerelleştirme |
| `DOC-` | Dokümantasyon ve topluluk |
| `QA-` | Test ve CI |
| `SYNC-` | Cihazlar arası senkron (ADR 0002, 2.0 hattı) |

**Öncelik:** `P0` yayın öncesi zorunlu · `P1` ilk 1–2 sürüm · `P2` orta vade · `P3` uzun vade / isteğe bağlı
**Efor:** `S` < 1 gün · `M` 1–3 gün · `L` 1–2 hafta · `XL` > 2 hafta (tek geliştirici, Linux makinede)

Her maddede mümkün olduğunca **Ne / Neden / Nerede / Nasıl / Kabul ölçütü / Bağımlılık** verilmiştir.

---

## 1. Yönetici özeti

Panora, teknik olarak sıradan bir "hobi pano yöneticisi"nin çok ötesinde: yerel x11rb/wayland-client backend'leri, TARGETS-önce gizlilik kapısı, XChaCha20-Poly1305 + Secret Service ile şifreli depolama, sandbox'lı systemd birimi, libadwaita popup, CLI, GNOME eklentisi ve 108 otomatik test var. Kod kalitesi ve güvenlik disiplini rakiplerin (CopyQ, GPaste, Pano) çoğundan yukarıda.

Buna karşılık **yayına hazır değil**; eksikler kodda değil, kodun etrafında:

1. **Kimlik sorunu.** Uygulama kimliği `io.panora.Panora`, D-Bus adları `io.panora.*`, eklenti UUID'si `panora@panora-clipboard.org` ve paket bakımcısı e-postası `panora@panora-clipboard.org`, projenin sahibi olmadığı alan adlarına dayanıyor. `panora.io` başka bir şirkete ait canlı bir site; `panora-clipboard.org` DNS'te çözülmüyor. Flathub ve extensions.gnome.org bu kimlikleri kabul etmez; ayrıca "Pano" adlı çok bilinen bir GNOME pano yöneticisi zaten var. Yayından önce karar şart (bkz. R-01, §7).
2. **GitHub deposu boş görünüyor.** Kökte `README.md` yok (`README-FIRST.md` GitHub tarafından gösterilmez), `LICENSE` kökte değil (GitHub lisansı algılamıyor), açıklama/etiket/sürüm/tag yok, community profile %0. Ziyaretçi projenin ne olduğunu anlayamıyor.
3. **Hiç gerçek masaüstünde çalıştırılmadı.** CI Linux'ta derleyip 108 testi geçiriyor; ama GUI, Wayland backend'i, keyring, IPC sunucusu, yapıştırma ve GNOME eklentisi için **sıfır** otomatik test var ve bunlar gerçek makinede ilk kırılacak yerler.
4. **Uygulama simgesi ve AppStream metadatası yok**; GNOME Yazılımlar/yazılım merkezleri listeleyemez, `.desktop` dosyası jenerik `edit-paste` simgesi kullanıyor.
5. **Bayat artefaktlar izleniyor.** 1.0/1.1 dönemine ait, terk edilmiş arboard/wl-clipboard mimarisini anlatan raporlar, var olmayan bir benchmark'a işaret eden `docs/benchmark.md`, boş `expected` dosyası, `test-artifacts/panod.pid`, `.typst-*.json` üretim dosyaları.
6. **Bilinen küçük hatalar** (B-01…B-27): `record_primary` için daemon yeniden başlatma gereksinimi, `max_mime_bytes` ile IPC yanıt sınırı uyumsuzluğu, GTK ana iş parçacığında bloklayan IPC, görsel satırları için tam boy payload aktarımı (küçük resim yok), GNOME ≤ 47'de tek MIME geri çağırma ile README iddiası arasındaki fark.

Yayın sonrası en değerli eklemeler (kullanıcı görünürlüğü × maliyet): odak kaybında kapanan popup ve imleç yakınında açılma, düz metin olarak yapıştırma kısayolu, Ctrl+1..9 hızlı seçim, küçük resimler + önizleme ötesi tam metin arama, hassas içerik sezgiseli ve otomatik süre dolumu, etiket/snippet'ler, gettext ile topluluk çevirisi, APT deposu + arm64 paket, wlroots'ta pano kalıcılığı, Wayland'de kaynak uygulama tespiti ve GNOME eklentisi için ayarlar penceresi.

---

## 2. Mevcut durum envanteri

### 2.1 Kod tabanı

| Bileşen | Dosyalar | Satır (yaklaşık) | Testler |
|---|---|---|---|
| `panora-core` | model, privacy, config, i18n, ipc, storage (db/blob/crypto), sync, backend trait | ~3 700 | 60 |
| `panod` | daemon, server, gnome (D-Bus), keyring, paste, backend/{x11,wayland,bridge} | ~4 500 + 163 (Xvfb entegrasyon) | 26 + 3 |
| `panora-gui` | main, window, details, settings, util, fixture | ~2 000 | 2 |
| `panora-cli` | main | 305 | 3 |
| GNOME eklentisi | `extension.js`, `metadata.json`, gschema | 661 | 0 (statik kontrol var) |
| Script'ler | `panora-doctor` (538), `e2e-test.sh` (862), `install.sh`, `build-deb.sh`, `make-kit.sh`, `security-check.sh` | ~2 100 | e2e manuel |
| **Toplam** | | **~13 200** | **108 otomatik** |

Test dağılımı: daemon 20, db 18, model 12, crypto 11, privacy 10, blob 8, ipc 6, i18n 4, config 4, cli 3, gnome 3, util 2, bridge 2, backend 1, wayland 1 (+3 X11 Xvfb). **Testsiz alanlar:** `panora-gui` (window/details/settings), `backend/wayland.rs` (bağlantı kurulmadan yalnızca 1 birim testi), `keyring.rs`, `server.rs` (IPC sunucusu), `paste.rs`, `extension.js`.

### 2.2 Var olan özellikler (kısa)

- **Yakalama:** X11 XFIXES olayları + INCR; Wayland `ext-data-control-v1` / `wlr-data-control-v1`; GNOME ≤ 47 için Shell eklentisi köprüsü (`PushMany`, yankı tespiti); PRIMARY isteğe bağlı; 23 MIME türü (`WANTED_MIMES`).
- **Gizlilik:** payload okunmadan MIME bayrağı kapısı, uygulama hariç listesi, özel mod, metadata sınırları; GNOME köprüsünde çağıran doğrulaması.
- **Depolama:** SQLite + FTS5 (500 karakterlik önizleme üzerinden), içerik adresli şifreli BLOB'lar (AAD ile bağlı, sürümlü zarf), tombstone, saklama sınırları (adet/gün), saatlik bakım.
- **GUI:** tek kolonlu Win+V benzeri panel, arama, 8 filtre çipi, sayfalama, canlı yenileme (1,5 s `Status` yoklaması), ayrıntı görünümü, ayarlar, tema, TR/EN, erişilebilirlik, RTL.
- **CLI:** list/search/copy/preview/pin/unpin/delete/clear/private/status/toggle/reload, `--json`.
- **Entegrasyon:** Super+V (eklenti `toggle-message-tray` çakışmasını çözer), D-Bus etkinleştirme, XTEST / Clutter sanal klavye / wtype / ydotool ile anında yapıştır, systemd user unit (sıkı sandbox, `RestrictAddressFamilies=AF_UNIX`).
- **Kurulum:** `KUR.sh` → `install.sh` (sürüm kapıları, rustup bootstrap, hazır `.deb` kullanımı), `panora-doctor`, `e2e-test.sh`, `make-kit.sh`.
- **CI:** fmt, clippy `-D warnings`, testler + Xvfb, eklenti statik güvenlik kontrolü, cargo-audit, cargo-deny, `.deb` (amd64) artefaktı, haftalık zamanlanmış çalışma.

### 2.3 GitHub deposu durumu (2026-09-19)

| Alan | Durum |
|---|---|
| Kök `README.md` | **Yok** (`README-FIRST.md` var, GitHub render etmez) |
| Kök `LICENSE` | **Yok** (`panora/LICENSE` var; GitHub `licenseInfo: null`) |
| Açıklama / topics / homepage | Boş |
| Release / tag | Yok |
| Issues / PR şablonları, CONTRIBUTING, CODE_OF_CONDUCT, SECURITY.md | Yok (community profile %0) |
| Branch protection | Yok |
| Dependabot / Renovate | Yok |
| Discussions | Kapalı |
| CI | `.github/workflows/ci.yml` — çalışıyor, yeşil |

### 2.4 Kimlik ve isimlendirme bulguları

| Kimlik | Kullanıldığı yer | Sorun |
|---|---|---|
| `io.panora.Panora` | GApplication id, `.desktop`, D-Bus service, eklenti | `panora.io` başka bir kuruluşun canlı sitesi. Flathub app-id için alan adı sahipliği ister. |
| `io.panora.GnomeBridge1`, `io.panora.GnomeShell1` | D-Bus adları | Aynı sorun. |
| `panora@panora-clipboard.org` | Eklenti UUID | `panora-clipboard.org` kayıtlı görünmüyor (DNS çözmüyor). extensions.gnome.org UUID'nin kontrol edilen bir ad alanı (ör. `ygkali.github.io`) olmasını önerir. |
| `Panora contributors <panora@panora-clipboard.org>` | `.deb` `Maintainer`, `Copyright` satırları | Debian politikası geçerli bir bakımcı adresi ister; e-posta çalışmıyor. |
| "Panora" adı | Her yer | "Pano" (oae/gnome-shell-pano, EGO #5278) GNOME'da en bilinen pano yöneticilerinden biri; ayrıca "Panora Exchange", "Panora" (panoratech) gibi yazılım ürünleri var. Hukuki engel görünmüyor ama arama/keşif karışıklığı olası. |

### 2.5 Doğrulama durumu

- CI'da derleme, clippy, 105 birim + 3 Xvfb entegrasyon testi geçiyor; `.deb` üretiliyor (glibc 2.39 tabanı).
- **Gerçek bir masaüstü oturumunda hiç çalıştırılmadı.** Zorin OS 18 (GNOME 46 Wayland) hedef makinesi henüz yok. İlk çalıştırma planı: `./KUR.sh` → oturumu yeniden aç → `panora-doctor` → `panora/scripts/e2e-test.sh --safe`.
- Wayland backend'i, keyring, GUI, yapıştırma ve eklenti yalnızca kod incelemesiyle doğrulanmış durumda.

---

## 3. Analizde tespit edilen hatalar ve tutarsızlıklar (B-xx)

Bu maddeler yol haritasındaki ilgili işlere bağlanmıştır; P0 olanlar yayından önce çözülmeli.

| ID | Bulgu | Etki | Nerede | Bağlı iş | Öncelik |
|---|---|---|---|---|---|
| B-01 | `history.record_primary` değişince PRIMARY izlemesi başlamıyor; `run()` ayarı bir kez okuyor | Ayar "anında uygulanır" iddiası bu alan için yanlış | `daemon.rs` `run()` | CAP-01 | P0 |
| B-02 | `max_mime_bytes` üst sınırı 256 MiB (`config.rs`), IPC yanıt üst sınırı 64 MiB (`ipc.rs` `MAX_RESPONSE_BYTES`); base64 ×4/3 nedeniyle ~48 MiB üstü payload'lar `Preview` ile okunamaz | Kullanıcı sınırı yükseltirse görseller GUI/CLI'da açılmaz | `config.rs validate`, `ipc.rs` | STO-08, R-08 | P0 |
| B-03 | GUI, `List`/`Status`/`Preview` çağrılarını GTK ana iş parçacığında bloklayarak yapıyor (`util::call`); ayrıntı görünümü 10 MiB'lık görseli ana iş parçacığında çözüyor | Yavaş daemon/büyük içerikte arayüz donar | `window.rs refresh()`, `details.rs show()` | UI-13 | P1 |
| B-04 | Her görsel satırı için tam payload IPC'den çekilip `PixbufLoader` ile çözülüyor; küçük resim önbelleği yok | 60 görselli sayfa = 60 tam aktarım + 60 iş parçacığı | `window.rs load_image_preview_async` | STO-03, UI-11 | P1 |
| B-05 | GNOME ≤ 47'de geri çağırma yalnızca tek MIME sunuyor (`St.Clipboard.set_content`), README "tüm biçimler" diyor | Zorin 18'de HTML+metin geri çağırma metne düşer | `extension.js SetClipboard`, `bridge.rs` | CAP-04, R-08 | P0 (doküman) / P1 (kod) |
| B-06 | ADR 0001 `gtk4-layer-shell` kullanılacağını söylüyor, bağımlılık/kod yok | Belge ile kod uyumsuz | `docs/adr/0001…`, `Cargo.toml` | UI-02, DOC-03 | P1 |
| B-07 | `docs/benchmark.md` `benches/fts.rs` ve `cargo bench` diyor; dosya yok | Yanıltıcı belge | `docs/benchmark.md` | STO-06, R-05 | P0 |
| B-08 | Bayat/çöp dosyalar izleniyor: `.typst-build-plan.json`, `.typst-content-manifest.json`, boş `expected`, `test-artifacts/panod.pid`, `reports/*` (1.0.0/1.1.0 hash'leri), 1.0/1.1 dönemi raporlar (`docs/system-status-test-report.md`, `video-test-report.md`, `ui-format-performance-report.md`, `pdf-system-status-report/`, `test-artifacts/`) | Yanlış mimari anlatan belgeler; depo kirli | kök ve `docs/` | R-05, DOC-03 | P0 |
| B-09 | `CHANGELOG.md` "Yayınlanmadı" bölümü var ama sürüm hâlâ 1.2.0; CI 1.2.0 etiketli ama içeriği farklı bir `.deb` üretiyor | Sürüm karmaşası | `Cargo.toml`, `CHANGELOG.md` | R-03 | P0 |
| B-10 | Uygulama simgesi yok (`Icon=edit-paste`), AppStream `metainfo.xml` yok, ekran görüntüsü yok | Yazılım merkezlerinde görünmez, jenerik simge | `packaging/io.panora.Panora.desktop` | R-06 | P0 |
| B-11 | Eklentide `prefs.js` yok; kısayol yalnızca `gsettings`/dconf ile değişir | Kullanıcı Super+V'yi arayüzden değiştiremez | `gnome-extension/` | INT-01 | P1 |
| B-12 | Popup odak kaybında kapanmıyor (Win+V kapanır) | UX beklentisi | `window.rs` | UI-01 | P1 |
| B-13 | `panora-cli` ve `panora-gui` `--version` desteklemiyor | Hata raporlarında sürüm belirsiz | `panora-cli/main.rs parse()` | CLI-01 | P0 |
| B-14 | `uninstall.sh` etkileşimli `read` kullanıyor; `--yes` yok | Otomasyonda takılır | `uninstall.sh` | PKG-09 | P2 |
| B-15 | Varsayılan hariç liste iki yerde tanımlı (`config.rs` `PrivacyConfig::default`, `privacy.rs` `DEFAULT_EXCLUDED_APPS`) ve içerikleri farklı | Kayma riski | `config.rs`, `privacy.rs` | SEC-10 | P2 |
| B-16 | FTS yalnızca 500 karakterlik önizlemeyi indeksliyor | Uzun metnin sonundaki kelime bulunamaz | `db.rs upsert_entry` | STO-02 | P1 |
| B-17 | Ana anahtar değişirse satırlar sessizce `[decryption failed]` gösteriyor; tespit/yönlendirme yok | Keyring sıfırlanan kullanıcı ne olduğunu anlamaz | `db.rs row_to_entry` | STO-09 | P1 |
| B-18 | `SCHEMA_VERSION = 1` yazılıyor ama migrasyon çerçevesi yok (`init()` yalnızca `CREATE IF NOT EXISTS`) | İlk herkese açık sürümden sonra şema değişimi kırıcı olur | `db.rs` | STO-01, R-09 | P0 |
| B-19 | GUI her 1,5 s `Status` yokluyor; olay kanalı yok | Gereksiz uyanma; başka istemciler için abonelik yok | `window.rs start_live_refresh` | STO-08 | P2 |
| B-20 | `install.sh`, `build-deb.sh`, `panora-doctor`, `e2e-test.sh` mesajları yalnızca Türkçe | Uluslararası kullanıcı kurulum çıktısını okuyamaz | script'ler | I18N-03, R-10 | P0 |
| B-21 | `.deb` `Maintainer` adresi geçersiz; `lintian` çalıştırılmamış | Debian politikası, güven | `packaging/build-deb.sh` | R-11 | P0 |
| B-22 | CI yalnızca amd64 `.deb` üretiyor; release iş akışı yok | arm64 kullanıcılar (Raspberry Pi, Apple Silicon VM) dışarıda | `.github/workflows/ci.yml` | PKG-01 | P1 |
| B-23 | Wayland backend'i `persist: false`; yorum "Mutter ve KWin saklar" diyor ama wlroots (Sway/Hyprland) kaynak istemci kapanınca panoyu **boşaltır** | Sway/Hyprland'de kopyalayan uygulama kapanınca pano kaybolur | `backend/wayland.rs capabilities()` | CAP-02 | P1 |
| B-24 | `argon2` ve `futures` workspace bağımlılığı olarak tanımlı, hiçbir crate kullanmıyor | Temizlik; `argon2` SEC-02 için hazır | `Cargo.toml` | SEC-02 | P3 |
| B-25 | Workspace depo kökünde değil `panora/` altında; `cargo install --git` çalışmaz, CI `working-directory` hileleri gerekir | Katılımcı sürtünmesi | depo düzeni | R-02 (karar D-2) | P0 (karar) |
| B-26 | Terminallerde anında yapıştır `Ctrl+V` gönderiyor; terminaller `Ctrl+Shift+V` bekler | Terminale yapıştırma çalışmaz veya yanlış davranır | `paste.rs`, `extension.js _sendCtrlV`, `x11.rs synthetic_paste` | INT-09 | P1 |
| B-27 | `e2e-test.sh` ve `test-local.sh` kopya yazmak için `xclip`/`wl-copy` gerektiriyor; bunlar `Suggests` bile değil | Temiz makinede testler SKIP olur | script'ler, `build-deb.sh` | QA-09 | P2 |

---

## 4. Yayın öncesi zorunlu işler (P0)

### R-01 · Kimlik kararı: ad, uygulama kimliği, alan adı, e-posta
- **Ne:** Projenin herkese açık kimliğini sabitle: uygulama adı, GApplication/D-Bus kimliği, eklenti UUID'si, bakımcı e-postası, telif hakkı sahibi.
- **Neden:** §2.4. Flathub `io.github.<kullanıcı>.<Uygulama>` biçimini GitHub'da barındırılan projeler için kabul eder; extensions.gnome.org UUID'de kontrol edilen ad alanı ister; `.deb` bakımcı adresi çalışmalı.
- **Seçenekler:**
  1. **Adı koru, kimlikleri GitHub ad alanına taşı (önerilen, en ucuz):** `io.github.ygkali.Panora`, D-Bus `io.github.ygkali.Panora.GnomeBridge1` / `.GnomeShell1`, UUID `panora@ygkali.github.io`, bakımcı `ygkali <kompansebuyucu@proton.me>`.
  2. Alan adı satın al (`panora.dev` vb.) ve `dev.panora.*` kullan.
  3. Yeniden adlandır ("Pano" karışıklığından tamamen kurtulmak için).
- **Nerede:** `crates/panora-gui/src/main.rs` (`APP_ID`), `crates/panod/src/gnome.rs` (BUS_NAME sabitleri), `gnome-extension/extension.js` (BRIDGE_NAME, HELPER_NAME, APP_BUS_NAME, APP_DESKTOP_ID), `metadata.json` (uuid), gschema id/path, `packaging/*.desktop`, `*.service`, `build-deb.sh` (EXT_UUID, Maintainer), `install.sh`/`uninstall.sh`/`panora-doctor`/`e2e-test.sh` (UUID ve bus adı sabitleri), `scripts/extension-security-check.mjs` (UUID ve ad denetimleri), README/INSTALL belgeleri, `Copyright` satırları.
- **Nasıl:** Tek commit'te toplu yeniden adlandırma; `gschemas.compiled` yolu değiştiği için eklenti kısayol anahtarı yeniden derlenir; eski UUID kurulu kullanıcılar için `postinst`'te eski dizini kaldırma + `gnome-extensions disable` notu.
- **Kabul:** `grep -rn "io.panora\|panora-clipboard.org"` sonuç vermez; `panora-doctor` yeni adlarla OK; CI yeşil.
- **Efor:** M.

### R-02 · Depo yüzü: README, LICENSE, açıklama, düzen
- **Ne:** Kökte İngilizce `README.md` (Türkçe `README.tr.md` ile), kökte `LICENSE` (kopya), depo açıklaması + topics (`clipboard-manager`, `gnome`, `wayland`, `rust`, `gtk4`, `libadwaita`, `linux`), homepage, social preview görseli, rozetler (CI, lisans, sürüm).
- **README içeriği:** 1 paragraf tanım, ekran görüntüsü/GIF, özellik listesi, destek matrisi (§README "Masaüstü uyumluluğu" tablosu), kurulum (deb indir / KUR.sh / kaynaktan), Super+V notu, gizlilik modeli özeti, CLI örnekleri, sorun giderme bağlantısı, katkı ve lisans.
- **Karar D-2 (depo düzeni):** workspace'i depo köküne taşı (`panora/` klasörünü düzleştir). Artıları: `cargo install --git` çalışır, CI `working-directory` hileleri kalkar, `KUR.sh` sarmalayıcıları doğrudan `install.sh` olur, GitHub dosya gezgini anlaşılır. Eksisi: tek seferlik yol güncellemeleri (`ci.yml`, `make-kit.sh`, memory notları). **Öneri: yap.**
- **Nerede:** kök, `.github/`, `README-FIRST.md` (kaldır/birleştir).
- **Kabul:** GitHub community profile'da README, LICENSE, description işaretli; `gh repo view` lisansı GPL-3.0 gösterir.
- **Efor:** M (ekran görüntüleri gerçek makine gerektirir; R-04'ten sonra tamamlanır).

### R-03 · Sürüm kesme ve release iş akışı
- **Ne:** "Yayınlanmadı" değişiklikleri **1.3.0** olarak kes (arayüz yeniden düzeni + köprü doğrulaması yeni özellik sayılır), git tag `v1.3.0`, GitHub Release; tag'e tetiklenen `release.yml`: `.deb` (amd64, arm64), `panora-<sürüm>-kit.tar.gz`, `SHA256SUMS`, SBOM, imza, CHANGELOG'dan notlar.
- **Nerede:** `Cargo.toml` `[workspace.package] version`, `CHANGELOG.md`, yeni `.github/workflows/release.yml`.
- **Nasıl:** Keep a Changelog biçimi + semver politikası (`docs/RELEASING.md`): protokol/DB biçimi değişirse minor, kırıcı ise major. `cargo-release` veya elle. arm64 için `runs-on: ubuntu-24.04-arm` (public depolarda ücretsiz).
- **Kabul:** `gh release view v1.3.0` artefaktları listeler; `dpkg -i` temiz Ubuntu 24.04 VM'de kurulur; `panora-cli --version` = 1.3.0.
- **Efor:** M. **Bağımlılık:** R-01, R-08, PKG-01.

### R-04 · Gerçek makinede doğrulama turu
- **Ne:** Zorin OS 18 (GNOME 46 Wayland) + en az bir GNOME 48+ (Debian 13 veya Ubuntu 25.10) + X11 oturumu (Zorin'de "GNOME on Xorg") üzerinde: `KUR.sh`, yeniden giriş, `panora-doctor`, `e2e-test.sh` (güvenli değil, tam), popup akışları (Super+V, arama, Enter, anında yapıştır, ayrıntı, ayarlar), keyring kilitli senaryosu, paket yükseltme (1.2.0 → 1.3.0), kaldırma.
- **Neden:** §2.5; ilk hatalar köprü/eklenti hattında beklenir.
- **Çıktı:** `docs/verification/2026-xx-zorin18.md` (PASS/FAIL tablosu, `panora-doctor --json` çıktısı), bulunan hatalar için issue'lar.
- **Kabul:** e2e tüm PASS; doktor 0 HATA; README destek matrisi gerçek sonuçlarla güncellenmiş.
- **Efor:** L (hata düzeltmeleri dahil). **Not:** Bu makinede (Windows, Smart App Control) derleme mümkün değil; doğrulama yalnızca Linux makinede yapılabilir.

### R-05 · Bayat belge ve artefakt temizliği
- **Ne:** B-07, B-08. Sil: `.typst-build-plan.json`, `.typst-content-manifest.json`, `expected`, `test-artifacts/panod.pid`, `reports/`. Arşivle veya sil: `docs/system-status-test-report.md`, `docs/video-test-report.md`, `docs/ui-format-performance-report.md`, `docs/ui-format-performance-plan.md`, `pdf-system-status-report/`, `test-artifacts/` (ekran görüntüleri yeniden çekilecek; `capture-*.sh` script'leri `scripts/` altına taşınabilir). `docs/benchmark.md`'yi STO-06 gerçekleşene kadar "ölçüm bekleniyor" olarak yeniden yaz veya kaldır. `docs/security-checklist.md`'de "Ortam engeli" satırını CI gerçeğiyle güncelle.
- **`.gitignore`'a ekle:** `.typst-*`, `*.pid`.
- **Kabul:** `git ls-files` yalnızca güncel mimariyi anlatan dosyaları listeler; `docs/README.md` (indeks) hangi belgenin ne olduğunu söyler.
- **Efor:** S.

### R-06 · Uygulama simgesi, AppStream metainfo, desktop dosyası
- **Ne:** Özgün simge (scalable SVG + symbolic varyant, GNOME HIG "app icon" şablonu), `packaging/<app-id>.metainfo.xml` (AppStream: name, summary, description, screenshots, releases, content_rating, url'ler, `developer_name`, lisans), `.desktop`'ta `Icon=<app-id>`, `X-GNOME-UsesNotifications=true`, `SingleMainWindow=true`.
- **Nerede:** `packaging/`, `build-deb.sh` (`/usr/share/icons/hicolor/scalable/apps/`, `/usr/share/metainfo/`), `postinst` (`gtk-update-icon-cache`).
- **Kabul:** `appstreamcli validate --pedantic` temiz; GNOME Yazılımlar'da kurulu uygulama simgesiyle görünür; `desktop-file-validate` temiz.
- **Efor:** M (simge tasarımı dahil).

### R-07 · Topluluk ve güvenlik belgeleri
- **Ne:** `SECURITY.md` (açık raporlama kanalı: GitHub private vulnerability reporting açık; destek penceresi), `CONTRIBUTING.md` (derleme, test, commit stili, DCO/imza politikası, dil), `CODE_OF_CONDUCT.md` (Contributor Covenant), `.github/ISSUE_TEMPLATE/` (bug: `panora-doctor --json` çıktısı zorunlu alan; feature; question → Discussions), `PULL_REQUEST_TEMPLATE.md`, `.github/dependabot.yml` (cargo + github-actions, haftalık), `CODEOWNERS`, Discussions açık, etiket seti (`area:gui`, `area:backend`, `platform:gnome-wayland`, …).
- **Kabul:** community profile %100.
- **Efor:** S–M.

### R-08 · Yayın öncesi hata düzeltmeleri
- B-01 (CAP-01), B-02 (config doğrulamasında `max_mime_bytes ≤ 40 MiB` sınırı **veya** IPC yanıt sınırını `max_mime_bytes × 4/3 + 1 MiB` olarak türet; kısa vadede ilki), B-05 (README/protocol-matrix'i gerçekle hizala; CAP-04 sonra), B-13 (`--version`), B-20 (I18N-03 asgari: İngilizce varsayılan), B-21 (bakımcı adresi).
- **Kabul:** her biri için birim testi veya e2e adımı.
- **Efor:** M toplam.

### R-09 · Disk biçimini dondur ve migrasyon çerçevesi
- **Ne:** STO-01'in çekirdeği: `meta.schema_version` okunur, sıralı migrasyon listesi çalışır (`migrations: &[(u32, &str)]`), migrasyon öncesi `history.db` yedeği (`history.db.bak-<sürüm>`), `PRAGMA integrity_check`, başarısızlıkta fail-closed ve açık hata. Zarf sürümü (`PNR1`) ve IPC protokol sürümü (2) için "kırıcı değişiklik = sürüm artışı" politikası `docs/COMPATIBILITY.md`'ye yazılır.
- **Neden:** İlk herkese açık sürümden sonra kullanıcı verisi kırılamaz; şu an `init()` şema değişikliklerini uygulayamaz (B-18).
- **Kabul:** "v1 DB → v2 migrasyonu" için test (fixture DB dosyası ile); eski zarf okuma testi zaten var.
- **Efor:** M.

### R-10 · Script ve tanı çıktıları için İngilizce varsayılan
- **Ne:** `install.sh`, `build-deb.sh`, `panora-doctor`, `e2e-test.sh`, `test-local.sh`, `uninstall.sh`, `make-kit.sh` mesajlarını İngilizce yap; Türkçe için `LANG=tr*` ise Türkçe tablo (basit `msg KEY` fonksiyonu ile iki dilli sözlük). Doktor durum etiketleri `OK/WARN/ERROR/INFO` (JSON'da sabit İngilizce anahtarlar).
- **Kabul:** `LANG=C ./install.sh` yalnızca İngilizce yazar; `LANG=tr_TR.UTF-8` Türkçe.
- **Efor:** M. (I18N-03 ile aynı iş.)

### R-11 · Paket kalitesi
- **Ne:** `lintian` temiz (veya bilinçli override'lar), DEP-5 makine okunur `copyright`, `changelog.Debian.gz`, man sayfaları (CLI-01), `Bugs:` alanı, `Vcs-Git`/`Vcs-Browser`, `Recommends: gnome-keyring | kwalletmanager`, `Suggests: xclip, wl-clipboard` (e2e için), postrm'de `systemctl --user disable` notu (kullanıcı verisi asla silinmez).
- **Kabul:** CI'da `lintian --fail-on error,warning dist/*.deb`.
- **Efor:** M.

### R-12 · Dal koruması ve zorunlu kontroller
- **Ne:** `main` için PR zorunlu, CI zorunlu (fmt-clippy, test, extension-security, audit, deny), force-push kapalı, linear history.
- **Efor:** S.

### R-13 · Test edilmemiş alanlar için asgari otomatik kapsama
- **Ne:** QA-01 (IPC sunucusu entegrasyon testi) ve QA-05 (CI'da gerçek `gnome-keyring-daemon` ile keyring testi) yayından önce; QA-02/03 sonra.
- **Neden:** Bu iki alan gerçek makinede ilk kırılacak, test edilebilir ve CI'da ucuz.
- **Efor:** M.

### R-14 · Hukuki hijyen
- **Ne:** `LICENSE` kökte; dosya başlıklarındaki "Panora contributors" telif sahibi ifadesi kalsın ama `AUTHORS`/`CONTRIBUTORS` dosyası ekle; REUSE uyumluluğu (`reuse lint`, `REUSE.toml`), `THIRD_PARTY_LICENSES.md` (`cargo about generate`), eklenti için GPL uyumlu lisans başlığı (var).
- **Kabul:** `reuse lint` geçer; CI'da `cargo about` çıktısı release artefaktı.
- **Efor:** S–M.

### R-15 · Eklenti yayın hazırlığı kararı
- **Ne:** Eklentiyi (a) yalnızca `.deb` ile dağıt, (b) ayrıca extensions.gnome.org'a (EGO) gönder. EGO gönderimi için: `prefs.js` (INT-01), `metadata.json` `version-name`, `gettext` alanı, `GLib.spawn_async` yerine `Gio.DesktopAppInfo.launch` (inceleme kolaylığı; mevcut spawn fallback'i EGO kurallarına aykırı değil ama incelemeci soru sorar), `session-modes` gerekçesi.
- **Öneri:** 1.3.0'da (a); 1.4'te (b).
- **Efor:** karar S; gönderim M.

---

## 5. Yayın sonrası geliştirme alanları

### 5.1 Yakalama ve backend'ler (CAP)

| ID | Başlık | P | Efor | Dosyalar |
|---|---|---|---|---|
| CAP-01 | `record_primary` canlı aç/kapa | P0 | S | `daemon.rs run()` |
| CAP-02 | wlroots'ta pano kalıcılığı (yeniden sunma) | P1 | M | `backend/wayland.rs`, `daemon.rs persist_after_owner_gone` |
| CAP-03 | Wayland'de kaynak uygulama tespiti (foreign-toplevel) | P1 | L | `backend/wayland.rs`, `Cargo.toml` |
| CAP-04 | GNOME ≤ 47'de çok biçimli geri çağırma | P1 | M | `extension.js`, `bridge.rs`, `gnome.rs` |
| CAP-05 | Yakalama filtreleri (uzunluk, boşluk, regex, tür, pencere başlığı) | P1 | M | `privacy.rs`, `config.rs`, `daemon.rs`, `settings.rs` |
| CAP-06 | Hassas içerik sezgiseli + maskeleme + otomatik süre dolumu | P1 | L | `privacy.rs`, `db.rs`, `daemon.rs`, GUI |
| CAP-07 | Sistem panosunu N saniye sonra temizleme (isteğe bağlı) | P2 | S | `daemon.rs` |
| CAP-08 | Tekilleştirme politikası seçenekleri | P2 | S | `db.rs upsert_entry`, `config.rs` |
| CAP-09 | Büyük payload'ları akış olarak yazma | P3 | M | `blob.rs`, backend `read` |
| CAP-10 | PRIMARY'ye geri çağırma (orta tık yapıştırma) | P2 | S | `ipc.rs Recall`, `daemon.rs` |
| CAP-11 | Ekran kilidinde yakalamayı duraklat | P1 | S | `server.rs`, `gnome.rs` (logind/ScreenSaver sinyali) |
| CAP-12 | Panoyu kimin değiştirdiğini kaydet (kaynak pencere başlığı, isteğe bağlı) | P3 | M | `x11.rs`, `extension.js`, `model.rs` |

**CAP-01 — `record_primary` canlı aç/kapa.** `apply_config` sonrasında `run()` döngüsüne bir "yapılandırma değişti" sinyali (`tokio::sync::watch`) gönder; döngü PRIMARY `watch()` alıcısını açar/kapar (`primary_rx = Some/None`). Kabul: `reloaded_exclusions_apply_live` benzeri bir test: `record_primary=false` başlat, `apply_config(true)`, PRIMARY olayı kaydedilir; tersine kapatınca kaydedilmez. e2e adımı ekle.

**CAP-02 — wlroots kalıcılığı.** wlroots kaynak istemci kapanınca seçimi boşaltır (`wl-clip-persist` bu yüzden var); Mutter/KWin saklar. `selection(null)` olayı geldiğinde ve son yakalama < 2 s içinde değilse ve önceki sahip biz değilsek → `OwnerGone` üret. Bilinçli temizleme (parola yöneticisi) ile ayırt edilemediği için **opt-in** (`history.persist_on_wayland = "auto" | "always" | "never"`; `auto` = bileşim yöneticisi adı wlroots ailesi ise açık; `XDG_CURRENT_DESKTOP` + `wl_registry` global'lerinden sezgisel). Gizlilik notu README'ye. Kabul: Sway headless testinde (QA-02) `wl-copy` süreci kapanınca `panora-cli list` üstteki kayıt `wl-paste` ile hâlâ okunabilir.

**CAP-03 — Wayland kaynak uygulama.** `wlr-foreign-toplevel-management-unstable-v1` (wlroots, KWin) ile `activated` toplevel'in `app_id`'sini izle; seçim olayında son etkin `app_id`'yi `source_app` yap (X11'in `_NET_ACTIVE_WINDOW` sezgiseliyle aynı). `ext-foreign-toplevel-list-v1` etkin durumu vermediği için tek başına yetmez. Mutter hiçbirini sunmuyor; GNOME'da eklenti zaten sağlıyor. `Capabilities::source_app` protokol varsa `true`. Kabul: Sway'de `excluded_apps` çalışır; ayarlardaki uyarı yalnızca protokolsüz bileşim yöneticilerinde görünür.

**CAP-04 — GNOME ≤ 47 çok biçimli geri çağırma.** `St.Clipboard.set_content` tek MIME sunar. Eklentide `Meta.SelectionSource` alt sınıfı (`GObject.registerClass`, `vfunc_get_mimetypes`, `vfunc_read_async/read_finish` → `Gio.Task` + `Gio.MemoryInputStream`) ile çok MIME'lı kaynak oluşturup `global.display.get_selection().set_owner(...)` ile sahiplen; yeni D-Bus metodu `SetClipboardMany(a(say))`, eski `SetClipboard` korunur; `bridge.rs offer` önce `SetClipboardMany` dener, `UnknownMethod`'da geri düşer. `extension-security-check.mjs`'teki metod listesi güncellenir. **Spike gerekli** (GJS'te async vfunc uygulaması). Kabul: Zorin 18'de HTML kaydı geri çağrılınca LibreOffice biçimli, terminal düz metin yapıştırır (e2e "HTML geri çağırma (iki biçim)" GNOME köprüsünde PASS).

**CAP-05 — Yakalama filtreleri.** `[privacy]` altına: `min_text_length` (varsayılan 1), `ignore_whitespace_only = true`, `ignore_patterns = ["^\\d{16}$", …]` (regex; `regex` crate, boyut sınırı ve derleme hatası doğrulaması), `capture_kinds = ["text","richtext","link","image","files","color"]`, `excluded_window_titles` (X11 `_NET_WM_NAME` ve GNOME eklentisi `window.get_title()`; bankacılık sekmeleri için). Değerlendirme yeri: MIME kapısından sonra, payload okunduktan sonra (metin gerektirir) — `Verdict::RejectFilter` yeni varyant. Ayarlar penceresinde "Gelişmiş filtreler" grubu. Kabul: birim testleri + e2e.

**CAP-06 — Hassas içerik sezgiseli.** Payload metni için: yüksek entropi + 20–128 karakter + boşluksuz (API anahtarı/parola), `sk-…`, `ghp_…`, `AKIA…` önekleri, kredi kartı (Luhn), IBAN (mod-97), JWT (`eyJ…`), özel anahtar blokları. Sonuç `entry.sensitive = 1` (yeni sütun → STO-01 migrasyonu): önizleme `••••` maskeli, FTS'e girmez, satırda kilit simgesi, `sensitive_ttl_minutes` (varsayılan 10) sonra otomatik silinir, `sensitive_policy = "mask" | "drop" | "store"`. `panora-cli list --json` `sensitive` alanı. Kabul: sezgisel için tablo tabanlı birim testleri (yanlış pozitif oranı: normal cümleler, URL'ler, hash'ler işaretlenmemeli), TTL testi.

**CAP-11 — Ekran kilidinde duraklat.** `org.freedesktop.login1.Session` `LockedHint` veya `org.gnome.ScreenSaver.ActiveChanged(b)`; kilitliyken `handle_event` erken döner (özel modu değiştirmez). Kabul: sinyal sahte-üretimi ile birim testi.

### 5.2 Popup ve ayarlar (UI)

| ID | Başlık | P | Efor | Dosyalar |
|---|---|---|---|---|
| UI-01 | Odak kaybında kapan (ayarlanabilir) | P1 | S | `window.rs` |
| UI-02 | İmleç/işaretçi yakınında konumlanma; wlroots'ta layer-shell | P2 | L | `window.rs`, `extension.js`, `Cargo.toml` |
| UI-03 | Düz metin olarak yapıştır kısayolu (Shift+Enter) ve satır eylemi | P1 | S | `window.rs`, `i18n.rs` |
| UI-04 | Ctrl+1..9 hızlı seçim, satır numarası ipuçları | P1 | S | `window.rs` |
| UI-05 | Çoklu seçim, birleştir, sırayla yapıştır | P2 | M | `window.rs`, `daemon.rs` |
| UI-06 | Yapıştırmadan önce düzenle / yeni kayıt olarak kaydet | P2 | M | `details.rs`, `ipc.rs` (`Store`) |
| UI-07 | Snippet'ler: adlandırılmış sabitler, etiketler, etiket çipleri, sabitli sıralama | P1 | L | `db.rs` (migrasyon), `ipc.rs`, `window.rs`, `settings.rs` |
| UI-08 | Metin dönüşümleri menüsü | P2 | M | `details.rs`, `daemon.rs` |
| UI-09 | İçeriğe duyarlı eylemler (bağlantı aç, QR kod, renk dönüştür, klasörü aç, görseli kaydet) | P1 | M | `window.rs`, `details.rs` |
| UI-10 | Kod algılama + eş aralıklı yazı tipi (+ isteğe bağlı vurgulama) | P2 | M | `model.rs`, `window.rs` |
| UI-11 | Küçük resimler, tembel yükleme, görsel boyutu bilgisi | P1 | M | STO-03 ile |
| UI-12 | Arama operatörleri, regex modu, eşleşme vurgusu | P1 | M | `db.rs`, `window.rs` |
| UI-13 | IPC'yi ana iş parçacığından çıkar; fark tabanlı liste güncelleme | P1 | M | `util.rs`, `window.rs`, `details.rs` |
| UI-14 | "Sıcak" popup modu (gizli kalıcı süreç) | P2 | M | `main.rs`, `window.rs`, `packaging/*.service` |
| UI-15 | Klavye: Home/End/PageUp/Down, listeden yazınca aramaya geç, F2 | P1 | S | `window.rs` |
| UI-16 | Satırları sürükle-bırak ile uygulamalara taşıma | P2 | M | `window.rs` (`gtk::DragSource`) |
| UI-17 | Silmeyi geri al (toast) | P1 | S | `window.rs`, `ipc.rs` (`Restore`), `db.rs` |
| UI-18 | Durum göstergesi (GNOME üst çubuk simgesi; KDE/Xfce için StatusNotifier) | P2 | M | `extension.js`, yeni `panora-tray` (isteğe bağlı) |
| UI-19 | İlk çalıştırma karşılama ekranı | P1 | S | `window.rs` |
| UI-20 | Ayarlar genişletmeleri (kısayol, otomatik başlatma, depolama kullanımı, tür bazlı yakalama, hassas TTL, popup davranışı) | P1 | M | `settings.rs`, `config.rs` |
| UI-21 | Rehberli hata sayfaları ("eklenti etkin değil", "keyring kilitli") | P1 | S | `window.rs`, `ipc.rs` (`Status.health`) |
| UI-22 | HTML/RTF önizlemesi (sınırlı Pango markup) | P3 | M | `details.rs` |
| UI-23 | Orca ile erişilebilirlik denetimi, yüksek kontrast, hareket azaltma | P2 | S | `window.rs` |
| UI-24 | Pencere boyutunu hatırla, çoklu ekran konumu | P2 | S | `window.rs`, `config.rs` |

**UI-01.** `window.connect_is_active_notify` → etkin değilse ve `ui.window.visible_dialog().is_none()` ise `close()`; `ui.popup_close_on_focus_out` ayarı (varsayılan açık). Kabul: fixture ile Xvfb'de pencere odağını başka pencereye alınca süreç çıkar.

**UI-02.** X11: `gdk::Display` işaretçi konumu → `window.present()` sonrası `gtk::Window::set_default_size` + `x11rb`'siz olarak GDK X11 `move_`; GNOME Wayland: eklenti `Activate` sonrası `Meta.Window.move_frame(true, x, y)` (odak penceresi Panora olduğunda; `global.get_pointer()`); wlroots: `gtk4-layer-shell` (feature `layer-shell`, `gtk4-layer-shell` crate, çalışma zamanında `is_supported()`), `Layer::Overlay`, anchor pointer köşesi. ADR 0001 buna göre güncellenir (B-06).

**UI-03.** `Shift+Enter` → `Recall { mime: Some("text/plain"), paste }`; satır eylemlerine "Düz metin" düğmesi (yalnızca `RichText`). Ayar: `ui.paste_plain_default`.

**UI-04.** İlk 9 satırın sol üstünde soluk `1…9`; `Ctrl+N` (ve `Alt+N`) → o satırı geri çağır. Filtre değişince numaralar güncellenir.

**UI-07.** Şema: `tags(id, name, color)`, `entry_tags(entry_id, tag_id)`, `entries.title TEXT NULL` (snippet adı), `entries.pin_order INTEGER`. IPC: `SetTitle`, `SetTags`, `ListTags`; `QueryRequest.tag`. GUI: sabitli satırlarda `F2` ad ver; etiket çipleri filtre satırının ikinci satırı; ayarlarda etiket yönetimi. CLI: `tag add/rm/list`. Kabul: migrasyon testi, sorgu testi, e2e.

**UI-09.** Bağlantı: "Tarayıcıda aç" (`gtk::UriLauncher`), "QR kod göster" (`qrcode` crate, çevrimdışı; telefona aktarma için senkronsuz köprü); renk: hex/rgb/hsl biçimleri arasında "… olarak kopyala"; dosya listesi: "Klasörü aç" (`FileLauncher::open_containing_folder`); görsel: "Farklı kaydet" (`FileDialog`), boyut/piksel bilgisi.

**UI-12.** Sorgu dilbilgisi: `kind:image app:firefox before:2026-09-01 after:7d pinned:yes tag:work "tam ifade"`; `db.rs query` ek koşullar; regex modu (`re:` öneki, `regex` crate, önizleme + tam metin üzerinde, sonuç sınırı). Eşleşme vurgusu: Pango markup ile `<b>` (kaçışlı).

**UI-13.** `util::call` yerine `util::call_async(request, callback)`: `std::thread` + `glib::MainContext::channel` (veya `glib::spawn_future_local` + `gio::spawn_blocking`); `refresh()` mevcut satırları `entry.id` + `last_seen_at` + `pinned` anahtarıyla karşılaştırıp yalnızca değişenleri yeniden kurar (`gtk::ListView` + `gio::ListStore`'a geçiş daha da iyi: sanal liste, 10 000 kayıtta da akıcı). `details.rs` görsel çözümü iş parçacığında. Kabul: 60 görselli sayfa açılışı ana iş parçacığını > 16 ms bloklamaz (tracing ölçümü).

**UI-14.** `--gapplication-service` ile başlayan süreç `hold()` ile hayatta kalır, pencere `hide()` olur; `Activate` ikinci çağrısı `present()`; bellek maliyeti (~40–60 MB) ayarlarda belirtilir; `ui.keep_popup_resident` (varsayılan kapalı). systemd `panora-gui.service` (isteğe bağlı) veya D-Bus etkinleştirme ile ilk açılışta başlar.

**UI-17.** `Delete` → tombstone zaten var; `collect_garbage` tombstone'ları hemen purge ediyor → `purge_tombstones(now - 15 s)` olarak değiştir, `Restore { id }` isteği (`deleted = 0`, FTS yeniden ekle), toast "Geri al" düğmesi.

**UI-19.** İlk açılışta (`~/.config/panora/first-run` yoksa) `adw::Carousel` 3 sayfa: Super+V, gizlilik modeli (keyring/şifreleme, ne kaydedilmez), oturum sınırlamaları (`Status.capabilities`'e göre dinamik: Wayland'de uygulama adı yok, GNOME ≤ 47'de eklenti gerekli).

**UI-21.** `Status` yanıtına `health: [{code, message}]` ekle (`extension_missing`, `keyring_locked`, `bridge_required`); GUI hata sayfası koda göre komut önerir ("gnome-extensions enable …" düğmesi `gio::Subprocess` ile çalıştırılabilir).

### 5.3 Komut satırı (CLI)

| ID | Başlık | P | Efor |
|---|---|---|---|
| CLI-01 | `--version`, çıkış kodları, man sayfası, kabuk tamamlama | P0/P1 | M |
| CLI-02 | `watch` (JSON satırları olay akışı) | P1 | M (STO-08 `Subscribe` gerektirir) |
| CLI-03 | `export` / `import` (şifreli arşiv), CopyQ/GPaste/Clipboard Indicator içe aktarma | P1 | L |
| CLI-04 | `pick` (dmenu/rofi/fuzzel/wofi ile), `--format` şablonu | P1 | S |
| CLI-05 | `config get/set/validate/edit`, `doctor` alt komutu | P2 | M |
| CLI-06 | `stats`, `wipe` (kriptografik imha), `lock`/`unlock` | P2 | M |
| CLI-07 | `copy-stdin` (stdin'i kaydet + panoya koy), `paste` alias | P1 | S |
| CLI-08 | `--json` çıktısı için JSON Schema ve sürüm alanı | P2 | S |

**CLI-01.** Karar: elle yazılmış ayrıştırıcıyı koru (bağımlılık yok) ve `--version`/`-V` ekle; ya da `clap` (derive, `clap_complete`, `clap_mangen`) — ikincisi man/tamamlama üretimini bedavaya getirir, ~1 MB ikili büyümesi kabul edilebilir. **Öneri: clap.** Çıkış kodları: 0 ok, 1 genel, 2 kullanım, 3 daemon yok, 4 bulunamadı. Kabul: `panora-cli --version` sürüm basar; `man panora-cli` `.deb` ile gelir; `panora-cli completions bash` çalışır.

**CLI-03.** `export --out backup.panora --passphrase-stdin`: Argon2id ile türetilen anahtarla XChaCha20 zarf; içerik: entries JSON + blob'lar (tar). `import` aynı içerik hash'lerini tekilleştirir. Dış biçimler: CopyQ (`copyq eval` JSON), GPaste (`gpaste-client history --raw`), Clipboard Indicator (`~/.cache/clipboard-indicator@tudmotu.com/registry.txt`). Kabul: gidiş-dönüş testi (export→import boş DB'ye = aynı kayıtlar).

**CLI-04.** `panora-cli pick --format '{id}\t{kind}\t{preview}' | fuzzel --dmenu | cut -f1 | xargs panora-cli copy --paste` örneği README'de; `--format` yer tutucuları `{id} {kind} {preview} {app} {age} {size}`; ayrıca `--null` ayırıcı.

### 5.4 Gizlilik ve güvenlik (SEC)

| ID | Başlık | P | Efor |
|---|---|---|---|
| SEC-01 | Ana anahtar döndürme (`rotate-key`) | P2 | M |
| SEC-02 | Geçmişi kilitleme: parola/PIN ile ikinci katman (Argon2id KEK), boşta kilit | P2 | L |
| SEC-03 | Panik silme kısayolu + kriptografik imha (`wipe`) | P2 | S |
| SEC-04 | `mlock`/`MADV_DONTDUMP`, `PR_SET_DUMPABLE=0` (ADR 0003 ertelenen madde) | P1 | S |
| SEC-05 | Günlük sızıntı testleri (payload/önizleme asla loglanmaz) | P1 | S |
| SEC-06 | Fuzzing (`cargo-fuzz`): IPC decode, `fts_query`, `strip_html`, `uri_list_preview`, `percent_decode`, MIME bayrağı ayrıştırma, zarf açma | P1 | M |
| SEC-07 | Resmi tehdit modeli belgesi + `SECURITY.md` + güvenlik sürüm süreci | P0/P1 | M |
| SEC-08 | Tedarik zinciri: `cargo-auditable`, CycloneDX SBOM, imzalı sürümler (minisign/cosign), yeniden üretilebilir derleme kontrolü, `cargo vet` (isteğe bağlı) | P1 | M |
| SEC-09 | systemd sertleştirme turu (`systemd-analyze security` hedef ≤ 2.0) | P1 | S |
| SEC-10 | Hariç liste tek kaynak (B-15), maskeli önizleme politikası | P2 | S |
| SEC-11 | IPC: `Hello`/sürüm pazarlığı, yanıt boyutu tutarlılığı (B-02) | P0/P1 | S |
| SEC-12 | Eklenti: `eslint-config-gjs` CI'da, spawn yerine `DesktopAppInfo.launch`, EGO inceleme notları | P1 | S |
| SEC-13 | Bağımsız güvenlik incelemesi (OSTIF / topluluk audit) | P3 | — |

**SEC-01.** `panora-cli rotate-key`: yeni anahtar üret → tüm blob ve önizlemeleri yeni anahtarla yeniden mühürle (geçici dizin + atomik rename; kesintiye dayanıklı: `meta.rotation_state`) → keyring öğesini güncelle → eski anahtarı sıfırla. Zarf sürümü değişmez. Kabul: kesinti simülasyonlu test.

**SEC-02.** İkinci katman: kullanıcı parolası → Argon2id (workspace'te `argon2` zaten tanımlı) → KEK; ana anahtar keyring'de KEK ile sarılı; daemon kilitliyken `List` yalnızca sayı döner, `Preview` reddedilir, GUI kilit ekranı; `lock_after_idle_minutes`. Biyometrik: `fprintd` üzerinden polkit ile "kilidi aç" (isteğe bağlı, P3).

**SEC-04.** `rustix::mm::mlock` ile `MasterKey` belleğini kilitle (RLIMIT_MEMLOCK küçük ama 32 bayt için yeterli); `prctl(PR_SET_DUMPABLE, 0)`; `madvise(MADV_DONTDUMP)`; systemd `LimitCORE=0`. Kabul: `cat /proc/<pid>/status | grep Dumpable` = 0.

**SEC-08.** Release iş akışında: `cargo auditable build`, `cargo cyclonedx`, `minisign -S` (özel anahtar GitHub secret), `SHA256SUMS` + `.minisig`; ikinci bir job aynı commit'i yeniden derleyip hash karşılaştırır (SOURCE_DATE_EPOCH, `--remap-path-prefix`); README'de doğrulama komutu.

### 5.5 Depolama, IPC ve performans (STO)

| ID | Başlık | P | Efor |
|---|---|---|---|
| STO-01 | Migrasyon çerçevesi, bütünlük kontrolü, FTS yeniden kurma (`fsck`) | P0 | M |
| STO-02 | Önizleme ötesi tam metin indeksleme (sınırlı), Türkçe katlama doğrulaması, trigram seçeneği | P1 | M |
| STO-03 | Sunucu tarafı küçük resimler (yakalamada üret, şifreli sakla) | P1 | M |
| STO-04 | Disk kotası ve tür bazlı sınırlar, boyuta göre LRU | P1 | M |
| STO-05 | DB bakımı: `PRAGMA optimize`, periyodik `VACUUM`, WAL checkpoint, öksüz blob taraması | P1 | S |
| STO-06 | Benchmark'lar (criterion) + CI'da bilgilendirici performans işi | P1 | M |
| STO-07 | Gerçek makinede bellek/başlangıç ölçümü ve hedefler | P1 | S |
| STO-08 | IPC v3: ikili çerçeveler veya fd geçirme, `Subscribe` olayları | P2 | L |
| STO-09 | Anahtar değişikliği tespiti (anahtar parmak izi `meta`'da) | P1 | S |
| STO-10 | Yedekleme/geri yükleme (CLI-03 ile) | P1 | — |

**STO-02.** Metin kayıtları için `entries_fts` ayrı `content` sütunu: ilk 64 KiB düz metin (şifresiz, DB dosyası 0600; belge notu güncellenir); `fts_query` `preview OR content` üzerinde. Türkçe: `unicode61` katlamasında `İ/ı/I/i` davranışını test et; gerekirse `tokenize='unicode61 remove_diacritics 2'` veya `trigram` (alt dize araması, 3+ karakter). Kabul: "İstanbul" kaydı `istanbul` ile bulunur; 600 karakterlik metnin son kelimesi bulunur.

**STO-03.** Yakalamada `image` türü için `image` crate ile ≤ 320 px PNG küçük resim → `blobs.put` → `entry_blobs(mime='x-panora/thumbnail')`; `Preview { id, thumbnail: true }` yalnızca onu döner; GUI listede küçük resim, ayrıntıda tam boy. Decode güvenliği: `image` crate sınırları (`Limits`), SVG için `librsvg` yerine küçük resim üretme (GTK'ya bırak). Kabul: 60 görselli sayfa IPC trafiği < 2 MB.

**STO-04.** `history.max_total_bytes` (varsayılan 512 MiB), `history.max_images` (200), `history.max_image_bytes`; `collect_garbage` boyut tabanlı LRU (sabitliler hariç); ayarlarda kullanım çubuğu (`total_size` zaten var).

**STO-06.** `crates/panora-core/benches/{fts,store,blob}.rs`, `crates/panod/benches/ipc.rs` (mock backend + Unix socket gidiş-dönüş); CI'da `cargo bench --no-run` derleme kontrolü + haftalık ölçüm artefaktı; `docs/benchmark.md` gerçek çıktıyla yeniden yazılır (B-07).

**STO-08.** Protokol 3: çerçeve = `u32 uzunluk + JSON başlık + ham payload`; büyük payload'lar için `memfd_create` + `SCM_RIGHTS` (rustix) ile fd geçirme (base64 ve 64 MiB sınırı sorunu kökten çözülür); `Subscribe` → daemon `revision` değiştikçe `{event: "changed", revision}` satırı (GUI yoklamayı bırakır, CLI `watch`). v2 istemcileri bir sürüm boyunca desteklenir (`Hello` ile pazarlık).

**STO-09.** `meta.key_fingerprint = blake3(key)[..8]`; açılışta uyuşmazlık → `Status.health = key_mismatch`, GUI "Geçmiş başka bir anahtarla şifrelenmiş" sayfası: "Yeni geçmişe başla (eskiyi arşivle)" veya "Keyring'i geri yükle" yönlendirmesi.

### 5.6 Masaüstü entegrasyonu (INT)

| ID | Başlık | P | Efor |
|---|---|---|---|
| INT-01 | GNOME eklentisi: `prefs.js`, gösterge simgesi, EGO yayını, her GNOME döngüsünde doğrulama | P1 | M |
| INT-02 | KDE Plasma: Klipper birlikte çalışma belgesi, KWallet testi, KRunner eklentisi | P2 | M |
| INT-03 | wlroots/Hyprland/Sway: layer-shell popup, örnek yapılandırmalar, `pick` entegrasyonu | P2 | M |
| INT-04 | Xfce/MATE/Cinnamon/LXQt: kısayol belgeleri, tepsi simgesi, test matrisi | P2 | S |
| INT-05 | Herkese açık, sürümlü D-Bus API'si (IPC'nin aynası) | P2 | M |
| INT-06 | systemd'siz sistemler için XDG autostart | P3 | S |
| INT-07 | Dosya yöneticisi kes/kopyala semantiği doğrulaması (`x-special/gnome-copied-files` `cut` satırı) | P1 | S |
| INT-08 | Uygulama uyumluluk matrisi (Chrome/Firefox/VS Code/LibreOffice/Telegram/Electron) | P1 | M |
| INT-09 | Terminallerde `Ctrl+Shift+V`; uygulama bazlı yapıştırma tuşu | P1 | S |
| INT-10 | Ekran kilidi/oturum değişimi davranışı (CAP-11 ile) | P1 | — |

**INT-01.** `prefs.js` (`ExtensionPreferences`, `Adw.PreferencesPage`): kısayol düzenleyici (`Gtk.ShortcutLabel` + yakalama diyaloğu), "üst çubukta gösterge" anahtarı, "yapıştır sonrası bildirim" anahtarı. Gösterge: `PanelMenu.Button` + menü (Geçmişi aç, Özel mod, Ayarlar); özel mod durumu `io.…GnomeBridge1` `PrivateMode` property'si (yeni). GNOME 49/50/51 için `shell-version` güncelle ve her sürümde `gnome-shell --headless` ile duman testi (QA-04). EGO gönderimi: `version-name`, `gettext-domain`, inceleme notları (neden `session-modes: zorin`, neden `St.Clipboard`).

**INT-05.** `io.github.ygkali.Panora1` (ad R-01'e göre) arayüzü: `List(a{sv}) → aa{sv}`, `Recall(x, b, s)`, `Pin`, `Delete`, `Clear`, `SetPrivate`, `Status`, sinyal `Changed(t revision)`; Unix soket iç protokol olarak kalır. Belge: `docs/dbus-api.md`. Üçüncü taraf eklentiler/Waybar modülleri için.

**INT-09.** Odak penceresinin `app_id`/`WM_CLASS` bilinen terminal listesindeyse (`org.gnome.Terminal`, `org.gnome.Ptyxis`, `org.kde.konsole`, `Alacritty`, `kitty`, `foot`, `org.wezfurlong.wezterm`, `xfce4-terminal`, `tilix`, `st`) `Ctrl+Shift+V` gönder; `paste_overrides = { "app_id" = "ctrl+shift+v" }` yapılandırması; X11'de `focused_app()` zaten var, eklentide `_popupHasFocus` mantığı genişletilir, Wayland (wtype) için `ext-foreign-toplevel` (CAP-03) gerekir. Kabul: gnome-terminal'e anında yapıştır çalışır (e2e adımı, GNOME'da).

### 5.7 Paketleme ve dağıtım (PKG)

| ID | Başlık | P | Efor |
|---|---|---|---|
| PKG-01 | Tag tetiklemeli release iş akışı: amd64 + arm64 `.deb`, kit, SHA256SUMS, SBOM, imza | P0 | M |
| PKG-02 | İmzalı APT deposu (GitHub Pages / Cloudsmith / OBS) → `apt upgrade` | P1 | M |
| PKG-03 | Gerçek Debian kaynak paketi (`debian/`, `dh-cargo` veya vendored), lintian temiz, Debian/Ubuntu resmi dahil edilme yolu | P2 | L |
| PKG-04 | Flatpak fizibilite spike'ı ve olası paket | P2 | XL |
| PKG-05 | AUR (`panora`, `panora-git`), Nix flake, Fedora COPR (rpm spec) | P2 | M |
| PKG-06 | `cargo install --git` desteği (depo düzleştirme, D-2) | P0 | S |
| PKG-07 | Dağıtım tabanı politikası ve test matrisi (Debian 13, Ubuntu 24.04/25.10, Zorin 18, Mint 22, Pop!_OS 24.04) | P1 | S |
| PKG-08 | Kit üretimi CI'da (mode bitleri korunarak), release'e ekleme | P1 | S |
| PKG-09 | Kaldırma/purge davranışı, `uninstall.sh --yes`, veri silme ayrı komut | P2 | S |
| PKG-10 | Yükseltme yolu testleri (1.2.0 → 1.3.0 → …: DB migrasyonu, birim yeniden başlatma, eklenti yenileme notu) | P1 | M |

**PKG-02.** `reprepro` veya `aptly` ile `gh-pages` dalında `deb [signed-by=/usr/share/keyrings/panora.gpg] https://ygkali.github.io/panora/apt stable main`; GPG anahtarı GitHub secret; release iş akışı depo indeksini günceller; README'de üç satırlık kurulum. Alternatif: Launchpad PPA (PKG-03 gerektirir).

**PKG-04 (Flatpak).** Zorluklar: (1) daemon Flatpak içinde systemd user unit kuramaz → `org.freedesktop.portal.Background` ile otomatik başlatma + `--command=panod`; (2) Wayland soketi (`--socket=wayland`) data-control için yeterli, X11 için `--socket=x11`; (3) Secret Service portal üzerinden (`org.freedesktop.secrets` session bus izni); (4) GNOME ≤ 47 köprüsü için eklenti ayrıca EGO'dan kurulur; (5) `RestrictAddressFamilies` benzeri kernel zorlaması yok (`--share=network` verilmez, eşdeğer). Önce 2 günlük spike: manifest + Zorin'de çalışır mı? Sonuç olumluysa Flathub başvurusu (R-01 kimliği gerekir).

### 5.8 Yerelleştirme (I18N)

| ID | Başlık | P | Efor |
|---|---|---|---|
| I18N-01 | gettext altyapısı (Rust: `gettext-rs`; eklenti: GNOME gettext; `.desktop`/metainfo: `xgettext`), Weblate projesi | P1 | L |
| I18N-02 | Yerel ayara duyarlı biçimlendirme (tarih, boyut), çoğul kuralları, Türkçe büyük/küçük harf | P2 | S |
| I18N-03 | Script/tanı mesajları İngilizce varsayılan + Türkçe (R-10) | P0 | M |
| I18N-04 | RTL doğrulaması (ar/he), Almanca gibi uzun dillerde yerleşim | P2 | S |

**I18N-01.** Mevcut `i18n.rs` (112 alan, iki katalog) → `gettext!("…")` çağrılarına dönüşür; `po/panora.pot`, `po/tr.po`; `.deb` `/usr/share/locale/<dil>/LC_MESSAGES/panora.mo`; `meson` gerekmez, `build-deb.sh` `msgfmt` çalıştırır. Ara adım (daha ucuz): `Strings` yapısını koruyup katalogları `.po`'dan üreten bir `build.rs` — çevirmen aracı uyumluluğu için yine `.po`. Hedef diller: en, tr, de, fr, es, pt-BR, ru, zh-CN (topluluk).

### 5.9 Dokümantasyon ve topluluk (DOC)

| ID | Başlık | P | Efor |
|---|---|---|---|
| DOC-01 | README EN/TR, ekran görüntüleri/GIF, özellik ve destek tabloları (R-02) | P0 | M |
| DOC-02 | Belge sitesi (mdBook, GitHub Pages): kullanıcı kılavuzu, SSS, sorun giderme, gizlilik modeli, `config.toml` referansı, CLI referansı, IPC/D-Bus protokol belgesi, mimari diyagramı, ADR indeksi, katkı ve sürüm süreci | P1 | L |
| DOC-03 | Bayat belge temizliği (R-05), ADR güncellemeleri (0001 layer-shell, 0002 D-Bus adı), `docs/README.md` indeksi | P0 | S |
| DOC-04 | Keep a Changelog + semver politikası + sürüm notu şablonu | P0 | S |
| DOC-05 | Issue/PR şablonları, etiketler, kilometre taşları (bu belgedeki sürümler), proje panosu | P0 | S |
| DOC-06 | Yönetişim: CoC, bakımcılar, karar günlüğü (ADR'ler devam) | P1 | S |
| DOC-07 | Lansman: This Week in GNOME, OMG! Ubuntu, r/gnome, r/linux, Hacker News "Show HN", AlternativeTo/Flathub listeleri, demo GIF | P1 | M |
| DOC-08 | `panora-doctor --report` (kimlik bilgisi arındırılmış tanı paketi) | P1 | S |
| DOC-09 | Video: 60 saniyelik tanıtım (mevcut `record-feature-tour*.sh` script'leri yeniden kullanılabilir) | P2 | M |

### 5.10 Test ve CI (QA)

| ID | Başlık | P | Efor |
|---|---|---|---|
| QA-01 | IPC sunucusu entegrasyon testi (MockBackend + gerçek Unix soket, sınırlar, peer UID) | P0 | M |
| QA-02 | Wayland backend testi: CI'da `sway` headless (`WLR_BACKENDS=headless`) + `wl-copy`/`wl-paste` | P1 | M |
| QA-03 | GUI duman testi: `--features fixture` + Xvfb + ekran görüntüsü artefaktı + AT-SPI ağacı dökümü | P1 | M |
| QA-04 | Eklenti: ESLint (gjs), `gnome-shell --headless` ile yükleme/enable testi | P2 | M |
| QA-05 | Keyring entegrasyon testi: CI'da `dbus-run-session` + `gnome-keyring-daemon --unlock` | P0 | M |
| QA-06 | Kapsama (`cargo-llvm-cov` → Codecov), MSRV işi (1.85), `cargo doc -D warnings`, `shellcheck`, `typos`, `cargo-machete` | P1 | S |
| QA-07 | Özellik tabanlı testler (`proptest`): `fts_query`, `percent_decode`, zarf, `wanted_order` | P2 | S |
| QA-08 | Gece VM işi: GNOME Wayland headless oturumda e2e (spike) | P3 | L |
| QA-09 | Manuel sürüm doğrulama matrisi ve kontrol listesi (`docs/RELEASING.md`), `xclip`/`wl-clipboard` `Suggests` | P1 | S |
| QA-10 | CodeQL (JS), OpenSSF Scorecard, Dependabot (R-07) | P1 | S |

**QA-01.** `crates/panod/tests/ipc_server.rs`: `tempdir` + `XDG_RUNTIME_DIR` override, `Daemon` MockBackend ile, `serve_client` task'ı, istemci `panora_core::ipc::client::call`; senaryolar: List/Recall/Pin/Delete/Clear/Status/Preview/ReloadConfig, 64 KiB + 1 bayt çerçeve reddi, 257. istek reddi, geçersiz JSON, farklı UID (root'suz test edilemez → `socket_uid` parametresini sahte vererek).

**QA-05.** `.github/workflows/ci.yml` `test` işine: `sudo apt-get install gnome-keyring`; `dbus-run-session -- bash -c 'echo -n "" | gnome-keyring-daemon --unlock --components=secrets; cargo test -p panod --test keyring -- --ignored'`; test: ilk çağrı anahtar üretir, ikinci çağrı aynı baytları döner, kilitli koleksiyonda hata.

### 5.11 Senkronizasyon (SYNC) — 2.0 hattı

| ID | Başlık | P | Efor |
|---|---|---|---|
| SYNC-01 | Fizibilite spike'ı: `iroh` boyut/bağımlılık etkisi, ayrı `panora-sync` ikilisi/paketi, feature flag | P3 | M |
| SYNC-02 | Eşleştirme (QR + kısa doğrulama kodu), cihaz listesi, grup anahtarı | P3 | XL |
| SYNC-03 | Seçici senkron (yalnızca sabitliler / yalnızca metin), Lamport/LWW çakışma kuralları (şema hazır) | P3 | L |
| SYNC-04 | Önce LAN-only mod (mDNS + QUIC), sonra relay | P3 | L |
| SYNC-05 | Android companion (Quick Settings tile, manuel gönderim) | P3 | XL |
| SYNC-06 | Yayın öncesi bağımsız güvenlik incelemesi; varsayılan kapalı; `RestrictAddressFamilies` yalnızca sync biriminde gevşetilir | P3 | — |

`SyncProvider`, `device_id`, `lamport`, `deleted` alanları hazır; `sync_active` `Status`'ta var. UI-09 QR kod özelliği, senkron gelmeden telefona tek yönlü aktarım sağlar.

---

## 6. Önerilen sürüm planı

| Sürüm | Tema | İçerik | Tahmini efor |
|---|---|---|---|
| **1.3.0** — "Herkese açık ilk sürüm" | Yayına hazırlık | R-01…R-15, CAP-01, CLI-01 (`--version`), STO-01, QA-01, QA-05, PKG-01, PKG-06, DOC-01/03/04/05, I18N-03, B-02/B-05(doküman)/B-13/B-20/B-21 | 4–6 hafta (gerçek makine doğrulaması dahil) |
| **1.4.0** — "Günlük kullanım" | UX | UI-01, UI-03, UI-04, UI-11 + STO-03, UI-12 + STO-02, UI-13, UI-15, UI-17, UI-19, UI-20, UI-21, UI-09, CAP-05, CAP-06, CAP-11, INT-09, CLI-04, CLI-07, STO-04, STO-05, STO-09, SEC-04, SEC-05, SEC-09, SEC-11, QA-03, QA-06, QA-10, DOC-08 | 6–8 hafta |
| **1.5.0** — "Platformlar" | Masaüstü genişliği | CAP-02, CAP-03, CAP-04, UI-02, UI-18, INT-01 (EGO), INT-02, INT-03, INT-04, INT-07, INT-08, PKG-02, PKG-05, PKG-07, PKG-08, PKG-10, I18N-01, QA-02, QA-04, DOC-02, DOC-07 | 6–8 hafta |
| **1.6.0** — "Sertleştirme" | Güvenlik ve ölçek | SEC-01, SEC-02, SEC-03, SEC-06, SEC-08, STO-06, STO-07, STO-08 (IPC v3) + CLI-02, CLI-03, UI-05, UI-06, UI-07, UI-08, UI-14, INT-05, PKG-03, QA-07, SEC-10 | 6–8 hafta |
| **1.7.x** | Cilalama | UI-10, UI-16, UI-22, UI-23, UI-24, CLI-05, CLI-06, CLI-08, CAP-07, CAP-08, CAP-10, PKG-04 (Flatpak), PKG-09, I18N-02/04, DOC-06/09 | sürekli |
| **2.0.0** — "Senkron" | ADR 0002 | SYNC-01…06 | 3+ ay |

Her sürüm için GitHub milestone açılır; bu tablodaki ID'ler issue başlıklarında kullanılır (`[UI-03] Paste as plain text shortcut`).

---

## 7. Açık kararlar (kullanıcı onayı gereken)

| ID | Karar | Seçenekler | Öneri |
|---|---|---|---|
| D-1 | Uygulama adı ve kimlikleri (R-01) | (a) Panora + `io.github.ygkali.*` (b) alan adı satın al (c) yeniden adlandır | **(a)**; "Pano" karışıklığını README'nin ilk cümlesinde açıkça ayrıştır |
| D-2 | Depo düzeni | workspace kökte / `panora/` altında kalsın | **Köke taşı** (`cargo install --git`, CI sadeleşir) |
| D-3 | Eklenti dağıtımı | yalnızca `.deb` / ayrıca EGO | 1.3'te `.deb`, 1.4–1.5'te EGO |
| D-4 | Varsayılan dil ve belge dili | TR-öncelikli / EN-öncelikli | **EN varsayılan**, TR tam çeviri (kod/commit zaten İngilizce) |
| D-5 | CLI ayrıştırıcı | elle yazılmış / `clap` | `clap` (man + tamamlama bedava) |
| D-6 | Hassas içerik varsayılan politikası (CAP-06) | maskele+TTL / hiç kaydetme / kaydet | **maskele + 10 dk TTL** |
| D-7 | Sürüm numarası | 1.2.1 / 1.3.0 | **1.3.0** (arayüz yeniden düzeni yeni özellik) |
| D-8 | Flatpak yatırımı | şimdi / spike sonra / hiç | spike (PKG-04) 1.5'ten sonra |
| D-9 | Senkron kapsamı | LAN-only önce / iroh relay / hiç | LAN-only spike, 2.0 |
| D-10 | GUI liste altyapısı | `FlowBox` kalsın / `ListView` + `ListStore` | UI-13 ile `ListView`'a geç |

---

## 8. Riskler

| Risk | Etki | Azaltma |
|---|---|---|
| Gerçek makinede köprü/eklenti hattında beklenmeyen hatalar | 1.3.0 gecikir | R-04'ü en başa al; hatalar için 1 haftalık tampon |
| GNOME 49/50/51'de `St.Clipboard`/`MetaSelection` API değişiklikleri | Zorin sonrası dağıtımlarda eklenti kırılır | QA-04, her GNOME sürümünde `shell-version` doğrulaması; GNOME 48+ zaten native yol |
| Tek bakımcı yükü | Roadmap yavaşlar | P0'ları küçük tut; CONTRIBUTING ile katkı kolaylaştır; "good first issue" etiketleri |
| Kimlik değişikliği mevcut kurulumları bozar | 1.2.0 kullanıcıları (şu an yok) | R-01 yayından **önce** yapıldığı için geçiş maliyeti sıfır |
| Şifreleme anahtarı kaybı (keyring sıfırlama) | Geçmiş okunamaz | STO-09 net mesaj, CLI-03 yedek, SEC-01 döndürme |
| Windows geliştirme makinesinde derleme yok | Değişiklikler yalnızca CI ile doğrulanır | Küçük PR'lar, CI'ı zorunlu kıl (R-12), Linux makine gelince R-04 |

---

## 9. Rakip karşılaştırması

| Özellik | Panora | CopyQ | GPaste | Pano (GNOME ext.) | Clipboard Indicator | cliphist | Win+V |
|---|---|---|---|---|---|---|---|
| Diskte şifreleme (AEAD + OS keyring) | ✓ | ✗ (düz) | kısmi (opsiyonel) | ✗ | ✗ | ✗ | ✗ |
| Parola yöneticisi bayrağı (payload okunmadan) | ✓ | kısmi | kısmi | ✗ | ✗ | ✗ | — |
| GNOME Wayland ≤ 47 yakalama | ✓ (eklenti) | ✗/kısmi | ✓ (eklenti) | ✓ | ✓ | ✗ | — |
| GNOME 48+/KDE/wlroots native data-control | ✓ | kısmi | ✗ | ✗ (GNOME) | ✗ (GNOME) | ✓ (wlroots) | — |
| X11 kalıcılık + INCR | ✓ | ✓ | ✓ | — | — | ✗ | — |
| Görsel / HTML / dosya listesi / renk | ✓ | ✓ | kısmi | ✓ | görsel | metin+görsel | ✓ |
| Tam metin arama (FTS) | ✓ (önizleme; STO-02 ile tam) | ✓ | ✓ | ✓ | ✓ | dış araç | ✗ |
| Sabitleme | ✓ | ✓ | ✓ | ✓ | ✓ | ✗ | ✓ |
| Etiket / snippet | UI-07 | ✓ (sekmeler) | ✗ | ✗ | ✗ | ✗ | ✗ |
| Düzenleme | UI-06 | ✓ | ✓ | ✗ | ✗ | ✗ | ✗ |
| Hızlı seçim (Ctrl+N) | UI-04 | ✓ | ✓ | ✓ | ✗ | — | ✗ |
| Anında yapıştır | ✓ | ✓ | ✓ | ✓ | ✓ | dış araç | ✓ |
| Betikleme / CLI | ✓ CLI | ✓ (JS) | ✓ CLI | ✗ | ✗ | ✓ | ✗ |
| Kaynak uygulama hariç listesi | ✓ (X11/GNOME) | ✓ | ✗ | ✗ | ✗ | ✗ | — |
| Sandbox'lı daemon (systemd) | ✓ | ✗ | ✗ | — | — | ✗ | — |
| Senkron | SYNC (2.0) | ✗ | ✗ | ✗ | ✗ | ✗ | ✓ (bulut) |
| Emoji/GIF paneli | ✗ (kapsam dışı) | ✗ | ✗ | ✗ | ✗ | ✗ | ✓ |
| Kod algılama/vurgulama | UI-10 | ✗ | ✗ | ✓ | ✗ | ✗ | ✗ |
| OCR ile görsel arama | ✗ (P3 adayı, `tesseract` isteğe bağlı) | ✗ | ✗ | ✗ | ✗ | ✗ | ✗ |

Panora'nın ayırt edici konumu: **güvenlik ve gizlilik odaklı, GNOME'da native görünen, her Wayland bileşim yöneticisinde çalışan** pano yöneticisi. Pazarlama mesajı bu üçlü üzerine kurulmalı; feature paritesi için UI-03/04/07/12 yeterli.

---

## 10. Ek

### 10.1 Analiz yöntemi
Tüm Rust kaynakları, eklenti, script'ler, paketleme dosyaları, ADR'ler, güvenlik belgeleri, CI iş akışı ve GitHub depo meta verileri (`gh api`) okundu; alan adı sahipliği HTTP/RDAP ile kontrol edildi; EGO inceleme kuralları, Flathub kimlik politikası, GitHub arm64 runner durumu ve rakip projeler web üzerinden doğrulandı. Hiçbir kod derlenmedi (bu makinede mümkün değil); bulgular statik incelemeye dayanır.

### 10.2 Kaynaklar
- Pano (GNOME pano yöneticisi): https://github.com/oae/gnome-shell-pano · https://extensions.gnome.org/extension/5278/pano/
- GNOME Shell eklenti inceleme kuralları: https://gjs.guide/extensions/review-guidelines/review-guidelines.html
- Flatpak sandbox izinleri: https://docs.flatpak.org/en/latest/sandbox-permissions.html
- xdg-desktop-portal pano yöneticisi portalı tartışması: https://github.com/flatpak/xdg-desktop-portal/issues/743
- ext-data-control-v1 / wlr-data-control durumu: https://github.com/bugaevc/wl-clipboard/issues/242
- GitHub arm64 runner'ları (public depolarda ücretsiz): https://github.blog/changelog/2025-08-07-arm64-hosted-runners-for-public-repositories-are-now-generally-available/
- Panora adını kullanan diğer ürünler: https://github.com/panoratech/Panora · https://panora.exchange/

### 10.3 Hızlı başlangıç: ilk 10 iş
1. D-1…D-10 kararlarını ver (özellikle D-1, D-2, D-7).
2. R-01 kimlik yeniden adlandırması (tek commit).
3. R-05 temizlik + `.gitignore`.
4. R-02 README/LICENSE/topics + D-2 düzleştirme.
5. R-08 hata düzeltmeleri (CAP-01, B-02, B-13) + R-10 İngilizce script çıktıları.
6. R-09/STO-01 migrasyon çerçevesi.
7. QA-01, QA-05 testleri; R-12 dal koruması.
8. R-06 simge + metainfo; R-07 topluluk dosyaları; R-11 lintian.
9. PKG-01 release iş akışı (amd64 + arm64) ve `v1.3.0-rc1` tag'i.
10. R-04 gerçek makine turu → hataları düzelt → `v1.3.0`.

---

## 11. Durum — 2026-09-20

Bu bölüm planın hangi kısmının uygulandığını kaydeder. `fd1d966` (yol haritası) sonrası commit'ler `main` dalında, CI yeşil; dal koruması açık. P0, 1.4.0 listesinin tamamı ve 1.5.0 listesi bitti; kalanların gerekçesi 11.2'de.

### 11.1 Tamamlanan

- **Yayın engelleri (P0):** R-01 kimlikler (`io.github.ygkali.Panora`, UUID `panora@ygkali.github.io`, bakımcı `ygkali`), R-02 README EN/TR + LICENSE + depo kökü (D-2), R-03 sürüm 1.3.0 + CHANGELOG, R-05 temizlik, R-06 simge/AppStream/ekran görüntüleri, R-07 topluluk dosyaları, R-08 hata düzeltmeleri (B-01, B-02, B-13, B-05 belge), R-09/STO-01 migrasyon çerçevesi (şema v4, `VACUUM INTO` yedek, anahtar parmak izi STO-09), R-10/I18N-03 İngilizce varsayılan betikler, R-11 lintian temiz `.deb`, **R-12 dal koruması** (12 zorunlu CI kontrolü, yöneticiler muaf, force-push ve silme kapalı), CI genişletmesi, PKG-01 release iş akışı (deb ×2, kit, SHA256SUMS, minisign, SBOM), PKG-06, CLI-01, QA-01, QA-02 (headless sway e2e), QA-05, SEC-07, SEC-08, SEC-11.
- **1.4.0 listesinin tamamı:** UI-01, UI-03, UI-04, UI-11+STO-03 (küçük resimler), UI-12+STO-02 (arama dilbilgisi `kind:` `app:` `pinned:` `before:`/`after:` `re:`, tırnaklı ifade, kalın eşleşme vurgusu; tam metin dizini, aksan/İ katlama), UI-13 (IPC ve görsel çözme işçi iş parçacığında; değişmeyen sayfa yeniden kurulmaz), UI-15, UI-17 (geri al), UI-19 (karşılama), UI-20 (otomatik başlatma, depolama kullanımı, tür bazlı yakalama, hassas TTL, odak davranışı), UI-21 (`Status.health`, `extension_missing` + Etkinleştir), UI-09 (bağlantı: tarayıcıda aç / QR kod; renk: hex/rgb/hsl kopyala; dosya listesi: klasörü aç; görsel: farklı kaydet + piksel boyutu), CAP-05 (uzunluk/boşluk/regex/tür filtreleri **ve** `excluded_window_titles`: X11 `_NET_WM_NAME`, eklenti `PushManyFrom`), CAP-06 (hassas içerik: anahtar/JWT/kart/IBAN/yüksek entropi; mask|drop|store, TTL), CAP-11, INT-09, CLI-04, CLI-07, STO-04 (`max_total_bytes`, `max_images`), STO-05 (bakım + öksüz blob), STO-09, SEC-04, SEC-05, SEC-09, SEC-11, QA-03 (ekran görüntüsü + AT-SPI ağaç dökümü `scripts/a11y-check.sh`), QA-06 (`typos`, `cargo-machete`, `cargo doc -D warnings`, llvm-cov kapsama işi), QA-10 (CodeQL JS+actions, OpenSSF Scorecard, Dependabot grupları), DOC-08 (`panora-doctor --report`).
- **1.5.0 listesi (bu tur):** CAP-03, UI-02, INT-02/INT-03/INT-04 (`docs/DESKTOPS.md`: masaüstü başına özellik matrisi, wlroots/KWin kaynak uygulama, layer-shell yerleşimi), PKG-02 (imzalı APT deposu: `packaging/apt-repo.sh`, `.github/workflows/apt-repo.yml`), QA-04 (eklenti ESLint: `gnome-extension/eslint.config.mjs`, CI adımı), PKG-10 (`scripts/upgrade-test.sh`: eski sürümün yazdığı geçmişi yeni daemon açıyor, kayıtlar ve dizin korunuyor, şema hedefe göç ediyor, göç öncesi yedek duruyor; şema 2→4 ile yerel olarak doğrulandı), PKG-07 (`docs/DISTRIBUTION.md`: resmi kanallar, test edilen dağıtım listesi, sürümleme ve destek penceresi, paketleyici notları), PKG-05 (`packaging/aur/PKGBUILD` + `.SRCINFO`, `packaging/nix/flake.nix`, `packaging/rpm/panora.spec`; üçü de **test edilmemiş başlangıç noktası** olarak işaretli), I18N-01 ara adım (katalog `po/panora.pot` + `po/tr.po`; `build.rs` aynı `Strings` yapısını üretiyor, eksik/boş/bayat girdi derlemeyi durduruyor), DOC-02 (mdBook sitesi `docs/book/`, gh-pages `/docs` altına; `config.toml` ve CLI referansları, arama dilbilgisi, gizlilik modeli), DOC-07 (`docs/launch.md` duyuru taslakları), INT-01 `prefs.js` (kısayol yakalama diyaloğu + `move-to-pointer`; B-11 kapandı).
- **Bağımlılıklar:** gtk4-rs 0.11 / libadwaita-rs 0.9, rusqlite 0.40, toml 1, qrcode 0.14.

### 11.2 Bilinçli olarak dışarıda bırakılan

| Madde | Neden |
|---|---|
| R-04 gerçek makine turu, `v1.3.0` etiketi | Zorin OS 18 makinesinde kullanıcı yapacak; etiket CI'da release iş akışını tetikler (`docs/RELEASING.md`). |
| PKG-02'nin canlıya alınması | Depo kodu ve iş akışı hazır; GitHub'da `APT_GPG_PRIVATE_KEY` / `APT_GPG_PASSPHRASE` secret'ları ve gh-pages için Pages ayarı kullanıcıya ait. |
| UI-18 durum göstergesi (StatusNotifierItem) | `ksni` **Unlicense** ile geliyor; `deny.toml` izin listesinde yok ve lisans politikasını tek bir P2 özelliği için genişletmek doğru takas değil. Ayrıca gösterge, sıkılaştırılmış daemon'a yeni bir D-Bus servis yüzeyi ekler ve ne CI'da ne WSL'de bir `StatusNotifierWatcher` var — doğrulanmadan sevk edilirdi. GNOME'da zaten ayrı bir eklenti gerekir. Elle yazılmış `com.canonical.dbusmenu` uygulaması (zbus zaten bağımlı) gerçek bir masaüstü turundan sonra yeniden değerlendirilecek. |
| INT-01 EGO gönderimi | `prefs.js` bu turda geldi, ama extensions.gnome.org gönderimi `version-name`, `gettext-domain`, inceleme notları ve her GNOME sürümünde duman testi ister; çalışan Shell oturumu olmadan yapılmaz. |
| CAP-04 (çok biçimli GNOME geri çağırma), INT-07, INT-08, STO-07 | Çalışan bir GNOME Shell oturumu gerekir; WSL/CI'da doğrulanamaz. |
| PKG-05 paketlerinin denenmesi | `makepkg`, `nix build` ve `rpmbuild` WSL'de yok; dosyalar bilerek "test edilmemiş" etiketiyle duruyor (`packaging/README.md`). |
| PKG-08 kit üretiminin CI'a taşınması | Kit `make-kit.sh` ile release iş akışında zaten üretiliyor; ayrı bir CI işi ilk release'ten önce kanıtlanacak bir şey eklemiyor. |
| I18N-01'in tamamı (`gettext-rs`, `.mo` kurulumu, Weblate) | Ara adım yeterli: çevirmen araçları `.po` okuyor, çalışma zamanı bağımlılığı yok. Yeni dil geldiğinde `.mo` yolu değerlendirilecek. |
| SEC-06 fuzzing, STO-06 benchmark, CLI-02/CLI-03, UI-14, PKG-04 Flatpak | 1.6+ hattı; başlanmadı. |

### 11.3 Öğrenilenler

- GitHub push koruması, gerçek jeton desenine uyan test dizgelerini (Slack `xoxb-…`) reddeder; testlerde açıkça sahte biçimler kullanılmalı.
- `xwininfo -root -children` boş ekranda da "0 children." yazar; ekran görüntüsü betiği pencere satırını bekler ve boş kare almayana kadar çeker.
- `adw::Carousel` içindeki sayfalar diyalog genişliğini almadı; karşılama ekranı `gtk::Stack` kullanır.
- `ossf/scorecard-action` için `v2` takma adı yok ve `v2.4.0` imajı artık çekilemiyor; tam sürüm etiketi (`v2.4.4`) gerekir.
- `typos` iki dilli dizgelerde Türkçe sözcükleri yakalar; `_typos.toml` bunları listeler ve Türkçe belgeleri atlar.
- Derlemeyi üreten kaynak tek olmalı: katalog `.po`'ya taşınırken üretilen dosya, eski elle yazılmış `Strings` ile alan alan karşılaştırıldı; aksi hâlde "aynı çıktı" iddiası doğrulanamazdı.
- `panod` ekransız başlamaz (`no display`), bu yüzden yükseltme testi Xvfb altında koşuyor; `cargo test --workspace` ise WSLg'nin `DISPLAY`'ini görünce X11 testlerini atlamak yerine yanlış sonuç veriyor — yerel koşularda `DISPLAY` temizlenmeli.
- `panora-cli preview <id>` biçim başlığı da yazar; boru hattında ham içerik için `--mime` şart.
- gh-pages'i iki iş akışı paylaşıyor (`apt/` ve `docs/`); her ikisi de dalı çekip yalnız kendi dizinini değiştiriyor ve aynı `concurrency` grubunda çalışıyor.
- mdBook `{{#include}}` ile depo kökündeki belgeleri çekebiliyor; belge sitesi ikinci bir kopya üretmiyor.
- `scripts/a11y-check.sh` elindeki **release** ikilisini kullanır; bayat bir ikili "satır etiketi yok" diye başarısız olur. Koşudan önce `cargo build -p panora-gui --features fixture --release` şart (CI zaten yapıyor).
- Yerel derleme/test için WSL2 Ubuntu 24.04 yeterli: `cargo test`, Xvfb altında X11/GUI/AT-SPI testleri, headless sway ile Wayland e2e, `gnome-keyring-daemon` ile keyring testi.

### 11.4 1.6.0 "Sertleştirme" — durum (2026-09-21, WSL2 oturumunda tamamlandı)

19 maddelik listenin **13'ü bitti** (aşağıdaki tablo), **6'sı bu oturumda bilinçli olarak
bırakıldı** çünkü bu ortamda (WSL2, ekransız) gerçekten doğrulanamazlardı — yapılmadan
"bitti" işaretlemek, brief'in "gerçekten çalıştır, iddia etme" kuralını ihlal ederdi:

- **STO-07** (boşta RSS, popup açılış süresi) — gerçek bir masaüstü oturumu ve fiziksel
  makine ister; WSL/CI'da ölçüm anlamsız. Kalıcı olarak atlandı, brief'in kendi
  kapsam notuyla uyumlu.
- **UI-05, UI-06, UI-07, UI-08, UI-14** — GTK/libadwaita arayüz işi; bu oturumun hiçbir
  aracı çalışan bir GNOME Shell/GTK oturumunda görsel doğrulama yapamıyor (Xvfb altında
  a11y ağacı dökülebiliyor ama "doğru görünüyor mu" sorusu insan gözü ister). SEC-02'nin
  kilit ekranı ve SEC-03'ün panik-silme kısayolu — ki bu beşin en güvenlik-kritik alt
  kümesi — ayrı takip görevleri olarak işaretlendi (`task_ea5cdd6e`, `task_fc2ab47d`);
  UI-05/06/07/08'in geri kalanı (görsel cilalama, düzen) henüz görev olarak bile
  açılmadı, gerçek bir GTK oturumunda oturup bakılması gerekiyor.

Geri kalan 13 madde gerçekten çalıştırılıp doğrulandı: her biri kendi commit'i, testi ve
`CHANGELOG.md` satırıyla — aşağıdaki tablo bunların dökümü. `cargo fmt --check`,
`cargo clippy --workspace --all-targets -- -D warnings` ve `cargo test --workspace`
(WSL2, `DISPLAY` temizlenmiş) her madde sonrası tekrar çalıştırıldı, hepsi yeşil.

| Madde | Durum |
|---|---|
| STO-08 (IPC v3) | **Bitti.** `crates/panora-core/src/ipc/v3.rs`: `u32` uzunluk + JSON başlık + ham payload çerçevesi; 8 KiB üstü payload'lar `memfd_create` + `SCM_RIGHTS` (rustix) ile fd olarak geçiyor, altında satır içi. `Hello` sürüm pazarlığı, `Subscribe` → `daemon.revision()` değiştikçe `Event::Changed` akışı (`tokio::sync::watch`). v2 istemcileri aynı soket üzerinde `peek_first_byte` (MSG_PEEK) ile ayırt edilip eskisi gibi sunuluyor (`panod/src/server.rs dispatch_client`). Birim + gerçek soket entegrasyon testleri (`crates/panod/tests/ipc_server.rs`): küçük/büyük/karışık payload round-trip, Hello, Subscribe (ilk olay + değişiklik olayı), v2/v3 aynı soketi paylaşıyor. `panora_core::ipc::client::Subscription` istemci tarafı yardımcı (CLI-02 bunun üstüne kurulacak). GUI'nin `window.rs start_live_refresh` yoklamasını `Subscribe`'a taşıması (B-19) bu maddenin kapsamına alınmadı — ayrı, GTK-taraflı bir iş; not düşüldü. |
| CLI-02 (`watch`) | **Bitti.** `panora-cli watch`: `Subscription` üzerinden bloklayıp her değişiklikte bir satır basıyor (`revision=N` veya `--json` ile ham `Event`). İlk satır her zaman mevcut revizyon, bu yüzden bağlanmadan hemen önceki bir değişiklik kaçmıyor. `list`/`search` ile birleştirmek çağrıcıya bırakıldı (`watch | while read ...`). |
| SEC-01 (`rotate-key`) | **Bitti.** `Database::rekey`/`BlobStore::rekey` (panora-core): her önizleme/blob eski anahtarla açılıp yeniyle yeniden mühürleniyor; her ikisi de idempotent (satır/dosya zaten yeni anahtar altındaysa no-op) — kesinti sonrası tekrar çağrı güvenli. Yeni anahtar önce keyring'in "pending" slotuna yazılıyor (hiçbir şey dokunulmadan önce), `meta.rotation_state` ile izleniyor, canlı keyring öğesi yalnızca db+blob başarıyla bitince değiştiriliyor (`panod/src/keyring.rs`, `Daemon::rotate_key`). `Status.health` yarım kalan rotasyonu `rotation_incomplete` koduyla bildiriyor. Gerçek `gnome-keyring-daemon` + `dbus-run-session` üzerinde, yarıda kesilmiş rotasyonu simüle edip devam ettiren test dahil, 2 entegrasyon testi (`crates/panod/tests/rotate_key.rs`, `scripts/keyring-test.sh` çalıştırıyor) + 5 birim testi (db/blob katmanında) yeşil. |
| SEC-02 (ikinci katman kilit) | **Bitti.** Kapsam bilinçli daraltıldı (kullanıcıyla netleştirildi): kilit yalnızca **çalışma zamanında** devreye giriyor; `panod` her zaman keyring'deki düz anahtarla unattended başlıyor (systemd ile reboot sonrası kimse orada olmadan). `panora_core::lock::LockSecret`: parola doğrulaması ve KEK türetimi **bağımsız** Argon2id tuzlarıyla (saklanan PHC doğrulayıcısı asla KEK olarak yeniden kullanılmıyor — modülün kendi dokümantasyonunda gerekçesi var). `Daemon::engage_lock/unlock/set_lock_password`; kilitliyken `List`→yalnızca sayı, `Preview`/`Recall`→reddedilir (`panod/src/server.rs handle_request`). `privacy.lock_after_idle_minutes` + 30 sn'lik ayrı bir ticker ile boşta kilit. Parola kurulunca canlı ana anahtarın parola-korumalı bir yedek kopyası keyring'e yazılıyor (`role=lock-backup`); günlük kilit akışının parçası değil, salt yedek. `panora-cli lock [set-password\|change-password\|remove-password]`, `panora-cli unlock` — parolalar her zaman stdin'den okunuyor, asla komut satırı argümanı değil. Testlerde gerçek bir hata yakalandı ve düzeltildi: Secret Service `SearchItems` sorguyu *alt küme* eşleştiriyor, `role` etiketi olmayan ana anahtar sorgusu SEC-01'in pending/SEC-02'nin lock-backup öğeleriyle çakışıp yanlış öğeyi döndürebiliyordu — üç öğe türüne de ayrı `role` eklendi. Kapsam dışı bırakılan: GUI kilit ekranı (GTK, bu oturumda görsel doğrulama yapılamadı) ve biyometrik/fprintd (roadmap'te zaten P3). 4 birim testi (daemon.rs, gerçek keyring gerektirmez) + 2 entegrasyon testi (ipc_server.rs, gerçek soket) + 3 gerçek-keyring testi (`crates/panod/tests/lock.rs`, `scripts/keyring-test.sh`) yeşil. |
| SEC-03 (panik silme + kriptografik imha) | **Bitti.** `Database::wipe` (`clear_all`'dan farklı: tombstone değil, gerçek `DELETE` — sabitlenmiş dahil, `entries`/`entry_blobs`/`entries_fts`/kilit-ve-rotasyon meta anahtarları, sonra `VACUUM`) + `BlobStore::wipe` (her blob dosyası silinir) + `Daemon::wipe`: önce veriyi siler, sonra taze bir anahtar üretip `rekey` ile canlı depolamayı ona geçirir (bellekte bile eski anahtar kalmaz), en son keyring'deki tüm öğeleri (ana + varsa pending + varsa kilit yedeği) taze anahtarla değiştirir — böylece silinen baytlara ne olursa olsun eski şifreli içerik geri döndürülemez. İkinci katman kilitten bağımsız çalışıyor (panik eyleminin önce kilidi açmayı gerektirmemesi kasıtlı). `panora-cli wipe --yes` (onay bayrağı zorunlu, `uninstall.sh --yes` kalıbıyla tutarlı). 2 birim testi (db/blob, gerçek keyring gerektirmez) + 1 gerçek-keyring testi (`crates/panod/tests/wipe.rs`, `scripts/keyring-test.sh`) yeşil. Kapsam dışı: GUI'de "panik silme kısayolu" (klavye kısayolu) — GUI kilit ekranıyla birlikte ayrı işe bırakıldı. |
| CLI-03 (export/import) | **Bitti.** `panora_core::backup`: `.panora` arşivi = magic + tuz + XChaCha20 zarf (mevcut `Cipher`'ı yeniden kullanır) → içinde tar: `entries.json` + içerik-adresli `blobs/<hash>` (aynı payload birden çok girdi paylaşıyorsa arşivde tek kopya). Anahtar doğrudan paroladan Argon2id ile türetiliyor, ayrı bir doğrulayıcı saklanmıyor (AEAD etiketinin açılamaması zaten "yanlış parola" cevabı). `Daemon::export`/`import`: import gizlilik kapısını atlıyor (arşiv kullanıcının kendi yedeği) ve `upsert_entry`'nin `UNIQUE(content_hash,selection)` çakışmasıyla canlı yakalamayla aynı şekilde tekilleştiriyor. **STO-08'in v3'ü gerçek bir kullanım buldu**: `Request::Export`/`Import` ham arşiv baytlarını v3 çerçevelemesiyle taşıyor (`v3.rs`'in payload çıkarma/geri-yükleme mantığı `Store`'un yanına `Import`/`Archive` için genişletildi) — v2'nin 64 MiB / base64 sınırına tıkanmadan büyük geçmişler taşınabiliyor. `panora-cli export`/`import` parolayı her zaman stdin'den okuyor. `panora-cli import-legacy <copyq\|gpaste\|clipboard-indicator>`: her öğe normal `Store`/gizlilik-kapısı yolundan geçiyor; ayrıştırma mantığı (CopyQ JSON, GPaste ham satırlar, Clipboard Indicator registry.txt — hem düz dizi hem `{contents:...}` nesne biçimi) örnek verilerle test edildi ama **gerçek araçlara karşı doğrulanamadı** (WSL'de kurulu değiller) — bilinçli, dokümante edilmiş sınır. Kabul testi (`export`→boş DB'ye `import` = aynı kayıtlar, sabitlenmiş dahil) hem doğrudan (daemon.rs, 3 test) hem gerçek v3 soketi üzerinden (INLINE_LIMIT'in 3 katı büyüklükte payload ile, fd-geçirmeyi gerçekten tetikleyerek; ipc_server.rs, 2 test) yeşil. |
| SEC-06 (fuzzing) | **Bitti**, roadmap'in tam listesinden kapsamı daraltılmış: `strip_html`/`uri_list_preview`/`percent_decode` adında fonksiyon kod tabanında yok (aspiratif isimler) — onun yerine gerçekten var olan, güvenlik açısından eşdeğer 7 hedef fuzzlanıyor: IPC v2 `decode`, IPC v3 çerçeve başlığı (STO-08, yeni `v3::fuzz_parse_request_header`), arama sorgu grameri (`search::parse`), FTS5 `MATCH` üretici (`fts_query`), depolama zarfı `open`/`open_with_aad`, gizli/concealed-type MIME bayrak kontrolü (`PrivacyEngine::has_secret_flag`, ADR 0003 — "MIME bayrağı ayrıştırma" burası), içerik sınıflandırma (`ClipboardData::classify`/`text`). `crates/panora-core/fuzz/` (`cargo-fuzz`, nightly + clang gerekiyor — WSL'e kuruldu). CI'da `fuzz-build` işi her hedefi derliyor (gerçek fuzzing koşusu değil — saatler sürer, CI bütçesine sığmaz); `scripts/fuzz-smoke.sh` yerelde kısa süreli koşu yapıyor. İlk duman koşusunda (~1.8M toplam çalıştırma, hedef başına 8-15 sn) çökme bulunmadı — bu sadece harness'ın sağlam olduğunu gösteriyor, gerçek bir fuzzing kampanyası (saatler/günler, `scripts/fuzz-smoke.sh`'nin çok üstünde) hâlâ kullanıcıya kalıyor. |
| SEC-08 (tedarik zinciri) | **Bitti**, brief'in daralttığı kapsamla: SHA256SUMS + minisign + SPDX SBOM zaten PKG-01'de (1.3.0) vardı, tekrar üretilmedi. Gerçekten eklenen: `packaging/build-deb.sh` artık `cargo auditable build` kullanıyor (her ikili kendi bağımlılık manifestini taşıyor, `cargo audit bin panod` kaynak ağacı olmadan çalışıyor) + `--remap-path-prefix` ile derleme makinesi yollarını temizliyor. Yeni `reproducible` release CI işi aynı commit'i ayrı bir runner'da tekrar derleyip `build` işinin ikilileriyle SHA256 karşılaştırıyor; `publish` yalnızca ikisi de geçince çalışıyor. `scripts/check-reproducible-build.sh` ile yerelde de tekrarlanabilir — **gerçekten çalıştırıldı**: `panod`/`panora-gui`/`panora-cli` şu an bağımsız derlemeler arasında bayt-bayt aynı çıkıyor. `cargo vet` roadmap'te zaten "isteğe bağlı" işaretli, bırakıldı (yüzlerce geçişli bağımlılığı denetlemek/muaf tutmak ayrı, süregelen bir iş). |
| STO-06 (benchmark'lar) | **Bitti.** `crates/panora-core/benches/{fts,store,blob}.rs` + `crates/panod/benches/ipc.rs` (criterion). `ipc.rs` gerçek sunucuyu ayrı bir OS iş parçacığında kendi runtime'ıyla çalıştırıyor (Daemon `Rc` taşıdığı için criterion'ın async/çok iş parçacıklı executor'ında çalışamaz) ve her ölçüm iterasyonu `panora-cli`'nin yaptığı gibi gerçek, bloklayan bir soket bağlantısı açıyor — geliştirirken bir gerçek hata yakalandı: `UnixListener::bind` runtime `block_on`'dan ÖNCE çağrılmıştı, "no reactor running" ile çöküyordu; içeri taşınarak düzeltildi. CI'da `bench-build` işi her push'ta yalnızca derliyor, haftalık `benchmark` işi (cron ile) gerçekten çalıştırıp `bench-results` artifact'i yüklüyor. `docs/benchmark.md` gerçek ölçümle yeniden yazıldı (B-07 kapandı) — bu oturumun WSL2 makinesinde: FTS5 önek araması (10k satır) ~1,1 ms, IPC gidiş-dönüş ~72 µs (ikisi de hedefin çok altında), 1 KiB kayıt saklama ~5,5 ms (5 ms hedefini hafif aşıyor, 20 ms tavanın altında — muhtemelen bu sanal makinenin disk gecikmesi, bare-metal doğrulamalı). Boşta RSS ve popup açılış süresi hâlâ "ölçüm bekleniyor" (STO-07, gerçek oturum gerekiyor). |
| SEC-10 (hariç liste + maskeli önizleme politikası) | **Bitti.** B-15: `config.rs`'in `excluded_apps` varsayılanı artık `privacy::DEFAULT_EXCLUDED_APPS`'tan üretiliyor, ayrı bir listeye kopyalanmıyor (önceki iki liste FARKLIYDI — `config.rs`'de `org.keepassxc`/`com.bitwarden`/`secrets` eksikti; `PrivacyEngine::new` gerçek listeyi zaten her zaman birleştirdiği için canlı bir açık değildi ama gerçek bir kayma riskiydi); kayma sürmesin diye bir regresyon testi eklendi. `sensitive_policy = "mask"`in yalnızca *önizlemeyi* maskelediği, asıl içeriğin geri çağırma/önizleme/export'ta hâlâ döndüğü artık README/README.tr/docs/book'ta açıkça yazıyor (sırrı erişilemez kılmak için `drop` gerekir). Bu arada fark edilen belge boşlukları da kapatıldı: `lock_after_idle_minutes` (SEC-02) hiçbir kullanıcı dokümanında yoktu, CLI referans sayfası `watch`/`rotate-key`/`lock`/`unlock`/`wipe`/`export`/`import`/`import-legacy`'den hiçbirini listelemiyordu. |
| INT-05 (D-Bus API) | **Bitti.** `crates/panod/src/dbus_api.rs`: `io.github.ygkali.Panora1` oturum veriyolunda (`/io/github/ygkali/Panora1`), `List`/`Recall`/`Pin`/`Delete`/`Clear`/`SetPrivate`/`Status` + `Changed(t revision)` sinyali. İkinci bir uygulama değil, ince bir çeviri katmanı: her çağrı Unix soketin zaten kabul ettiği aynı `Request`'e dönüşüyor (`panora_core::ipc::client::call` üzerinden, `panod` kendi IPC istemcisi oluyor — `panora-cli` gibi), böylece gizlilik kapıları/şifreleme/tekilleştirme mantığı tek yerde kalıyor. `Changed` sinyali CLI-02'nin `watch`'ının kullandığı AYNI `Subscribe` akışını arka planda bir `spawn_blocking` görevinde dinleyip yayınlıyor. Bilinçli olarak DIŞARIDA bırakılan: payload içeriği (`Preview`), export/import, kilit parolası, anahtar rotasyonu/wipe — oturum veriyolu başka süreçlerin izleyebildiği bir yayın ortamı, bunların hiçbiri için özel 0600 Unix soketten daha iyi bir yer değil. `docs/dbus-api.md` (mdBook'a da eklendi). Gerçek bir oturum veriyoluna karşı test edildi (`crates/panod/tests/dbus_api.rs`, `scripts/dbus-test.sh`, CI'da yeni `dbus-api` işi): tam round-trip (Status/List/Pin/Recall/Delete) + `Changed` sinyalinin gerçekten bir capture'da tetiklendiği, 4 ardışık koşuda kararlı. |
| PKG-03 (Debian kaynak paketi) | **Bitti**, brief'in daralttığı kapsamla: gerçek `debian/` (control, changelog, copyright, source/format, rules, postinst/prerm/postrm, `panora.links`, `panora.lintian-overrides`) — standart Debian araçlarıyla (`dpkg-buildpackage`, `sbuild`, `pbuilder`) derlenebiliyor, `packaging/build-deb.sh`'nin yaptığı aynı yerleştirmeyi (ikililer, kullanıcı systemd birimi, desktop dosyası, D-Bus servis dosyası, metainfo, ikonlar, man sayfaları, shell tamamlamaları, GNOME Shell eklentisi) `debian/rules`'ta elle senkronize tutuyor. `debian/rules override_dh_installsystemd`'i boş bırakıyor çünkü `panod.service` bir *kullanıcı* birimi (debhelper'ın sistemd entegrasyonu sistem birimlerini hedefliyor) — postinst/prerm bunun yerine oturum açmış her kullanıcı için `systemctl --user` ile doğrudan yeniden başlatıyor/durduruyor (build-deb.sh ile aynı mantık); `panora.lintian-overrides` bunun neden `maintainer-script-calls-systemctl` uyarısını haklı olarak bastırdığını belgeliyor. Gerçek hatalar geliştirirken yakalandı ve düzeltildi: (1) man sayfası symlink'i (`panora.1.gz` → `panora-gui.1.gz`) `dh_auto_install` sırasında oluşturulmuştu — `dh_compress` asıl dosyayı sıkıştırıp yeniden adlandırdığında sarkan symlink'e dönüşüyordu; `debian/panora.links` (debhelper'ın kendi, `dh_compress`'ten SONRA çalışan `dh_link` mekanizması) ile düzeltildi. (2) `debian/panora.lintian-overrides` yanlışlıkla çalıştırılabilir bit ile oluşturulmuştu — `dh_lintian` çalıştırılabilir config dosyalarını script olarak çalıştırmayı DENİYOR, hata veriyordu. (3) bu depo `core.fileMode = false` ile çalışıyor (Windows checkout), bu yüzden `debian/rules`/`postinst`/`prerm`/`postrm` git'e ilk eklendiğinde varsayılan 100644 modla kaydedilmişti — `debian/rules` çalıştırılabilir OLMADAN `dpkg-buildpackage` hiç çalışamaz; `git update-index --chmod=+x` ile düzeltildi ve doğrulandı (`git ls-files -s debian/`). Gerçek doğrulama: WSL'de yerel bir ext4 üzerinde (DrvFs/9p bağlı `/mnt/c` Unix izin bitlerini kalıcı tutmuyor — Windows tarafında `chmod` her zaman no-op, dosyalar hep `rwxrwxrwx` görünüyor, bu yüzden doğrulama için yerel dosya sistemine kopyalandı) tam bir `dpkg-buildpackage -b -us -uc` koştu, `panora_1.3.0_amd64.deb` üretti; `lintian -v` üzerinde SIFIR uyarı/hata (tamamen lintian-temiz). Kapsam dışı bırakılan (brief'in kendi sınırlaması): Debian/Ubuntu resmi arşivlerine dahil olma yolu (ITP süreci) — insan/manuel bir süreç, bu oturumda otomatikleştirilmedi; `docs/DISTRIBUTION.md`'ye not eklendi. |
| QA-07 (özellik tabanlı testler) | **Bitti.** Roadmap'in dört hedefinin hepsi: `search::fts_expression` (`fts_query`), zarf (`Cipher::seal`/`open` + `_with_aad` çiftleri), `percent_decode`, `wanted_order` — `crates/panora-core/proptest-regressions/` commit'e dahil. **Gerçek bir hata bulundu ve düzeltildi**: arama metninde bir `\0` (NUL) baytı GEÇERSİZ arama hatasına yol açıyordu — SQLite'ın FTS5 sorgu-metni ayrıştırıcısı `MATCH` argümanını, uzunluk önekli TEXT olarak gelmesine rağmen NUL-sonlandırmalı bir C dizesi gibi tarıyor, bu yüzden gömülü NUL taramayı tırnak ortasında kesiyor ve SQLite "unterminated string" (sonlanmamış dize) hatası döndürüyordu — arama kutusunda kaza sonucu bir NUL'a rastlayan (ör. bir dosyadan yapıştırılan ikili veri, ya da IPC üzerinden hatalı biçimlendirilmiş bir istemci) her sorguyu tamamen bozuyordu. `search::fts_expression` artık `"` ile aynı yerde `\0`'ı da kırpıyor (`crates/panora-core/src/search.rs`). Diğer üç hedefte hata bulunmadı ama gerçek değişmezler doğrulandı: zarf'ın rastgele düz metin/AAD için round-trip'i ve rastgele/bozuk baytlarda asla panik atmaması (yalnızca hata); `percent_decode`'un `%` içermeyen girdide birebir geçmesi ve rastgele UTF-8'in tam `%XX` kodlamasından geri kurtarılması; `wanted_order`'ın yalnızca tanınan MIME türlerini tutması, tercihe göre sıralanması, yinelenmemesi, en fazla bir düz-metin türü tutması ve idempotent olması. |

### 11.5 1.7.x "Cilalama" — durum (başladı 2026-09-21)

1.6.0 `main`'e birleştirildikten hemen sonra, aynı oturumda: 1.7.x listesinden bu ortamda
(WSL2, ekransız) gerçekten inşa edip test edilebilecek üç madde — CAP-07, CAP-08, CAP-10.
UI-10/16/22/23/24, PKG-04 (Flatpak) ve GTK'ye dokunan geri kalan her şey bilinçli olarak
bu turun dışında; gerçek bir GTK oturumu gerektiriyorlar.

| Madde | Durum |
|---|---|
| CAP-07 (N saniye sonra panoyu temizle) | **Bitti**, ama üç gerçek hata düzeltilerek. `privacy.clear_clipboard_after_seconds` (varsayılan 0, kapalı). `ClipboardBackend::clear(selection)` yeni bir trait metodu (varsayılan "desteklenmiyor", X11/Wayland gerçek uyguluyor, GNOME bridge yok). `Daemon::recall` sonunda `schedule_clipboard_clear` bir `tokio::spawn` görevi kuruyor. **Hata 1 (tasarım):** ilk tasarım `read_targets` ile "hâlâ aynı MIME listesi mi" diye bakıyordu — ama iki farklı düz metin kopyası AYNI MIME listesini (`text/plain` vb.) sunar, bu yüzden birinin yerine geleni yanlışlıkla silerdi; `Daemon::capture_generation` (her gerçek pano değişikliğinde `handle_event`/`handle_gnome_data`/`recall` içinde artan bir sayaç) ile değiştirildi — zamanlayıcı ateşlediğinde sayaç hâlâ recall anındaki değerdeyse temizler. **Hata 2 (X11):** `release_selection`, `SetSelectionOwner(NONE)` gönderip `flush()` çağırıyordu ama sunucunun isteği GERÇEKTEN işlediğini beklemiyordu — gerçek Xvfb testinde ~her seferinde başarısız oluyordu (kapatma öncesi hâlâ eski sahip görünüyordu); `.check()` ile senkron bir gidiş-dönüşe zorlanarak düzeltildi. **Hata 3 (en ciddisi, yalnızca gerçek headless sway koşusunda yakalandı, mock testlerde DEĞİL):** Wayland'de panoyu temizlemek, seçimin boşalmasına yol açıyor — ki bu, panod'un KENDİ kalıcılık özelliğinin (`persist_on_wayland`) "kaynak uygulama çıktı" diye yorumladığı AYNI olay; sonuç: temizlemeden hemen sonra panod SİLİNEN girdiyi geri sunuyordu. `Daemon::clearing_deliberately` bayrağı (temizlemeden hemen önce set edilir, `handle_event`'in `OwnerGone` dalındaki BİR SONRAKİ olayda tüketilir) gerçek bir çıkışın kalıcılığını bozmadan bunu düzeltiyor. `crates/panod/src/daemon.rs` birim testleri (MockBackend, gerçek zamanlayıcı — `tokio::time::pause`/`advance`'ın bağımsız `tokio::spawn` görevlerini güvenilir sürmediği görülüp gerçek `sleep`'e geçildi) + `scripts/wayland-e2e.sh`'e iki yeni bölüm (8: temizleme gerçekten oluyor mu, 9: bkz. CAP-10) — headless sway üzerinde uçtan uca doğrulandı. |
| CAP-10 (PRIMARY'ye geri çağırma) | **Bitti.** `Request::Recall` yeni bir `to: Selection` alanı kazandı (varsayılan `Clipboard`, `#[serde(default)]` — mevcut JSON çağıranlar etkilenmiyor). `panora-cli copy <id> --primary`; `--paste` PRIMARY ile birlikte sessizce yok sayılıyor (PRIMARY yapıştırması için klavye kısayolu yok, yalnızca orta tık). `Selection` artık `#[derive(Default)]` (`#[default]` `Clipboard`). `MockBackend` gerçekçi hale getirildi: `offered` artık `Selection` anahtarlı bir `HashMap` (öncesinde tek bir alan her iki seçimi de karıştırıyordu — CAP-10'un asıl amacı olan "PRIMARY'ye yazmak CLIPBOARD'a dokunmamalı" hiçbir zaman gerçekten test edilemezdi). `scripts/wayland-e2e.sh` bölüm 9, `wl-paste --primary` ile gerçek bir sway üzerinde doğrulandı. |
| CAP-08 (tekilleştirme politikası) | **Bitti**, kapsam roadmap'in "S" efor tahminine göre daraltıldı: gerçek bir "her kopya kendi satırı" politikası `UNIQUE(content_hash, selection)` şema kısıtını kaldıran bir şema göçü isterdi (gerçek M/L efor); bunun yerine `history.duplicate_policy`: `bump` (varsayılan, mevcut davranış — aynı içeriğin yeniden kopyalanması satırı tazelenmiş bir zaman damgasıyla en üste taşır) veya `ignore` (satır yine aynı içeriğe tekilleşir, asla ikinci bir satır olmaz, ama konumu ve zaman damgası dokunulmadan kalır — sık kopyalanan bir şeyin listenin başını sürekli işgal etmesini istemeyenler için). `Database::upsert_entry` yeni bir `bump_on_duplicate: bool` parametresi alıyor; içe aktarma (CLI-03) her zaman `true` geçiyor (bir yedekten geri yükleme canlı bir yeniden-kopyalama değil). |
| Yan bulgu: `panora-gui --features fixture` derlemiyordu | Yukarıdaki işi doğrulamak için `scripts/wayland-e2e.sh` çalıştırılırken ortaya çıktı: `crates/panora-gui/src/fixture.rs`, SEC-01'den beri (bu oturumun 1.6.0 kısmından) hiç derlenmemiş — `StatusData`'nın `app_locked`/`lock_password_set` alanları ve o zamandan beri eklenen dokuz `Request` varyantı (`RotateKey`, `Lock`, `Unlock`, `SetLockPassword`, `Wipe`, `Export`, `Import`, `Hello`, `Subscribe`) fixture'a hiç yansıtılmamıştı — çünkü normal `cargo build`/`cargo test --workspace` döngüsü `--features fixture` GEÇMİYOR ve bu depoya ait `origin/main`'e o zamandan beri hiç push yapılmamıştı (CI'nin "yeşil" görünmesi bu yüzden yanıltıcıydı — test ettiği commit hâlâ 1.6.0 öncesiydi). Düzeltildi: eksik iki `StatusData` alanı eklendi, dokuz eksik `Request` varyantı için CLI/D-Bus-özel işlemler olduklarını (popup hiçbirini göndermiyor) açıklayan tek bir `_ => Err(...)` kolu eklendi. **Ders:** `--features fixture` derlemesi normal doğrulama döngüsüne dahil değil; her IPC `Request`/`ResponseData` değişikliğinden sonra elle `cargo build -p panora-gui --features fixture` çalıştırılmalı, ya da CI'ya ayrı bir adım eklenmeli (bu oturumda eklenmedi — gelecekteki bir iş). |
