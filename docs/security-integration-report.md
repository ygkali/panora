# Panora güvenlik standardı entegrasyon raporu

**Yazar:** ygkali
**Proje:** Panora v1.0.0  
**Tarih:** 19 Ağustos 2026  
**Kapsam:** Debian tabanlı Linux, GNOME, X11/Wayland, yerel şifreli clipboard geçmişi

## Yönetici özeti

Panora'nın güvenlik modeli, açık kaynak pano yöneticilerindeki pratikler ile freedesktop Secret Service, OWASP kriptografik depolama/anahtar yönetimi ve RustSec tedarik zinciri kontrolleri karşılaştırılarak güçlendirildi. CopyQ ve GPaste gibi projelerde görülen parola yöneticisi hariç tutma, gizli clipboard işaretlerini dikkate alma ve plain-text geçmiş sızıntısını azaltma yaklaşımı Panora'nın privacy engine'ine genişletildi [1] [2]. Secret Service'in collection, item, session, locked/unlocked ve prompt semantiği Panora keyring katmanına uygulanarak anahtar deposu kullanılamadığında sessiz anahtar yenileme yerine fail-closed davranış sağlandı [3].

Uygulama katmanında yeni sürümlü şifreli zarf, XChaCha20-Poly1305 authenticated encryption, BLOB ve preview için associated data, BLOB hash doğrulaması, atomic private file write, 0700/0600 izin sertleştirmesi, Unix peer UID doğrulaması, IPC frame/request sınırları, metadata boyut sınırları ve GNOME extension statik güvenlik kontrolleri eklendi. OWASP, kriptografik tasarımın tehdit modelinden başlamasını, AEAD ve özel key-management sistemlerinin kullanılmasını, anahtarların veriden ayrılmasını ve yaşam döngüsünün planlanmasını önerir [4] [5].

> **Sonuç:** Panora'nın kod ve CI güvenlik seviyesi önceki sürüme göre anlamlı biçimde artırıldı; ancak bu çalışma bağımsız penetrasyon testi, güvenlik sertifikasyonu veya tüm masaüstü ortamları için uçtan uca garanti değildir. Özellikle X11 clipboard API'si, bazı kaynak uygulamalarda gizli MIME metadata'sını payload okunmadan sağlamadığı için açık bir pre-read sınırlamasına sahiptir.

## Araştırma yöntemi

Araştırmada pano yöneticilerinin resmi açık kaynak depoları, freedesktop Secret Service spesifikasyonu, OWASP'ın Cryptographic Storage ve Key Management Cheat Sheet belgeleri ile RustSec'in resmi tooling açıklaması kullanıldı. RustSec, `cargo-audit`'in Cargo.lock'u advisory veritabanına karşı taradığını; `cargo-deny`'nin advisory, lisans, kaynak, duplicate dependency ve ban politikalarını kapsadığını; CI ve düzenli taramaların kullanılabileceğini belirtir [6].

| Kaynak/pratik | Güvenlik dersi | Panora karşılığı |
|---|---|---|
| CopyQ | Parola yöneticisi kaynaklarını ve hassas içerikleri geçmişten dışlama yaklaşımı | Varsayılan uygulama hariçleri, secret MIME marker'ları ve private mode |
| GPaste | Parola öğelerini düz metin geçmişinden uzak tutma ve gizli içeriği göstermeme yaklaşımı | Privacy gate payload okunmadan önce; hiçbir rejected entry DB/FTS/UI'ye ulaşmıyor |
| Secret Service | Session, collection, item, locked/unlocked ve prompt yaşam döngüsü | Keyring item arama, secret session kapatma, prompt'ta fail-closed |
| OWASP Cryptographic Storage | Tehdit modeli, AEAD, key/data ayrımı ve depolama katmanı seçimi | Versioned XChaCha20-Poly1305 envelope, AAD, Secret Service key |
| OWASP Key Management | Generation, storage, use, rotation, compromise recovery, destruction | Zeroized `MasterKey`, keyring metadata, versionli zarf ve migration read yolu |
| RustSec | Advisory taraması, lisans/bans ve scheduled CI | `cargo-audit`/`cargo-deny` CI job'ları ve haftalık schedule |

Ayrıntılı kaynak notları `docs/security-research-notes.md`, boşluk analizi `docs/security-gap-analysis.md`, kontrol tablosu `docs/security-checklist.md` dosyalarında tutulur.

## Entegre edilen kontroller

### Hassas clipboard politikası

`PrivacyEngine` artık aşağıdaki gizli MIME ve marker varyantlarını case-insensitive ve MIME parameter toleranslı biçimde tanır: KDE password manager hint, macOS concealed type, Qt clipboard viewer ignore ve bunların yaygın varyantları. Varsayılan hariç uygulamalar KeePassXC, Bitwarden, 1Password, GNOME Secrets ve paket/desktop-id biçimlerini kapsar. Girdi metadata'sı için 128 MIME, MIME başına 256 byte ve kaynak uygulama adı için 256 Unicode scalar sınırı vardır. Sınır dışı veya bozuk metadata `RejectMalformedMetadata` ile reddedilir.

Bu kontrolün önemli özelliği kararın payload okunmadan önce verilmesidir. X11 ve bazı Wayland watcher implementasyonlarının değişiklik tespiti için sınırlı pre-read yapabildiği açıkça belgelenmiştir; GNOME Shell bridge'de privacy gate daemon içinde ikinci kez uygulanır. Panora bu nedenle gizlilik politikasını yalnızca GUI'ye değil, daemon'ın capture ve GNOME bridge trust boundary'sine yerleştirir.

### Secret Service keyring

Keyring katmanı `SearchItems` için Secret Service'in standart `(unlocked, locked)` yanıtını ve bazı eski GNOME Keyring sürümlerinin yalnızca unlocked object array döndürmesini destekler. Locked item bildirimi varsa daemon fail-closed olur. Keyring servisi, collection veya D-Bus session kullanılamıyorsa yeni bir yerel anahtar üretip mevcut geçmişi erişilemez bırakmaz; uygulama kontrollü hata ile durur. `CreateItem` interactive prompt döndürdüğünde prompt otomatik onaylanmaz ve operasyon hata verir. Secret session işlem sonunda kapatılmaya çalışılır.

### Versioned AEAD envelope ve AAD

Yeni şifreli veri formatı aşağıdaki yapıdadır:

```text
[4-byte magic PNR1][1-byte version][24-byte XChaCha nonce][ciphertext + Poly1305 tag]
```

Magic ve version kullanımı, legacy nonce-first formatın ilk byte'ı ile yeni zarfın karıştırılmasını önler. XChaCha20-Poly1305 için her yazımda yeni random nonce kullanılır. BLOB ciphertext'i `panora/blob/v1/<content-hash>` associated data ile kendi content-addressed adını doğrular. SQLite preview'i `panora/entry-preview/v1/<content-hash>` associated data ile kayda bağlanır. Yanlış hash/context ile açma başarısız olur.

Eski yerel history'ler için legacy format yalnızca okunurken uyumluluk fallback'i olarak kabul edilir; yeni yazımlar her zaman versioned envelope kullanır. Bu yaklaşım eski veriyi otomatik silmeden migration imkânı sağlar, fakat key rotation için gelecekte explicit re-encryption/migration komutu eklenmesi gerekir.

### Dosya ve path güvenliği

BLOB path'i yalnızca tam 64 karakter lowercase hexadecimal BLAKE3 hash kabul eder. Geçersiz hash path traversal veya path panic oluşturmadan reddedilir. BLOB root ve shard dizinleri Unix'te 0700; veritabanı, BLOB, device-id ve temporary dosyalar 0600 ile oluşturulur. BLOB yazımı `create_new`, 0600 temporary dosya, `sync_all` ve atomic rename kullanır; concurrent dedup yarışında mevcut doğru dosya korunur. SQLite parent data directory 0700'e çekilir; böylece `-wal`/`-shm` yan dosyaları da kullanıcıya özel directory içinde kalır.

### IPC erişim kontrolü ve kaynak sınırları

Unix socket oluşturulduktan sonra 0600 izin uygulanır ve Linux'ta `SO_PEERCRED` üzerinden bağlanan peer UID, socket sahibinin UID'siyle karşılaştırılır. JSON-lines protokolünde tek frame 64 KiB, bağlantı başına istek sayısı 256 ve liste sorgusu en fazla 500 sonuç ile sınırlandırılır. Aşırı frame açık ve sınırlı bir hata yanıtıyla sonlandırılır; istemci flood'u daemon'ın belleğini sınırsız büyütemez.

### GNOME extension ve tedarik zinciri

`gnome-extension` için metadata/schema, Node syntax ve statik güvenlik kontrolü eklendi. Kontrol; beklenen UUID/shell-version alanlarını, yalnızca session D-Bus sınırını ve extension içinde network URL, `fetch`, `XMLHttpRequest`, `WebSocket`, `eval` veya dynamic `Function` primitive bulunmamasını doğrular. CI ayrıca bu extension kontrollerini ve `cargo-audit`/`cargo-deny` job'larını çalıştırır. CI haftalık scheduled advisory taraması yapar; release build audit ve deny job'larına bağlıdır.

## Doğrulama sonuçları

| Kontrol | Sonuç | Kanıt |
|---|---:|---|
| `cargo fmt --all -- --check` | Geçti | Final validation |
| `cargo clippy --workspace --all-targets -- -D warnings` | Geçti | Final validation |
| `cargo test --workspace` | Geçti | 60 panora-core, panod, GUI/CLI ve sync testleri dahil |
| Crypto roundtrip/tamper/wrong-key/truncated | Geçti | `storage::crypto` testleri |
| AAD context binding | Geçti | BLOB ve preview AAD testleri |
| BLOB path traversal | Geçti | Invalid hash regression test |
| BLOB/DB private permissions | Geçti | Unix permission tests |
| MIME secret marker ve metadata bounds | Geçti | PrivacyEngine testleri |
| GNOME extension static security | Geçti | `node scripts/extension-security-check.mjs` |
| IPC oversized frame | Geçti | Entegrasyon çıktısı: `IPC request exceeds the 64 KiB limit` |
| X11 + Secret Service daemon integration | Geçti | Daemon `backend=x11`, Secret Service, CLI status/list/private akışı |
| Release build | Geçti | `cargo build --release --workspace` |
| Debian package | Geçti | `dist/panora_1.0.0_amd64.deb` |
| Local `cargo-audit` | Ortam eksik | CI job zorunlu; sandbox'ta executable kurulu değil |
| Local `cargo-deny` | Ortam eksik | CI job zorunlu; sandbox'ta executable kurulu değil |

Final Debian paketi SHA-256 değeri:

```text
4a626c9f93ee3420b414ffafdfb3a00e36a25d369f35bdd65f2834c204fa293d  dist/panora_1.0.0_amd64.deb
```

## Kalan riskler ve önerilen sonraki adımlar

Birincil kalan risk X11 pre-read davranışıdır. X11 backend, her kaynak uygulamanın secret MIME hedeflerini güvenilir biçimde açmadığı durumlarda privacy gate'den önce sınırlı text okuması yapabilir. Bu nedenle X11'de uygulama hariç listesi ve kullanıcı özel modu önemini korur. Uzun vadeli çözüm X11 için TARGETS/XFixes tabanlı event akışı ve kaynak uygulama metadata'sı kullanılabilen güvenilir bir backend'e geçmektir.

İkinci risk, sandbox ortamında gerçek GNOME Shell 46–51, gerçek Wayland compositor çeşitliliği ve locked Secret Service collection ile tam matrix testinin yapılamamasıdır. Bu senaryolar CI'da gerçek GNOME session/container veya dağıtım test makinelerinde ayrıca koşulmalıdır.

Üçüncü konu anahtar rotation'dır. Şu anda format sürümü ve migration-read uyumluluğu vardır; kullanıcı kontrollü master-key rotation, eski key id'leriyle yeniden şifreleme, yedekleme ve compromise recovery işlemleri henüz ayrı bir komut olarak sunulmamaktadır. Bu özellik senkronizasyon modülü etkinleştirilmeden önce tasarlanmalıdır.

Son olarak, RustSec ve cargo-deny CI'da tanımlı ve release dependency olarak zorunludur; sandbox yerelinde bu araçların executable'ları bulunmadığı için yerel rapor bunu geçici ortam eksikliği olarak kaydeder. CI'da advisory veya lisans ihlali bulunduğunda release job'ı ilerlememelidir.

## Referanslar

[1]: https://github.com/hluk/CopyQ "CopyQ resmi GitHub deposu"
[2]: https://github.com/Keruspe/GPaste "GPaste resmi GitHub deposu"
[3]: https://specifications.freedesktop.org/secret-service/latest-single/ "Freedesktop Secret Service API specification"
[4]: https://cheatsheetseries.owasp.org/cheatsheets/Cryptographic_Storage_Cheat_Sheet.html "OWASP Cryptographic Storage Cheat Sheet"
[5]: https://cheatsheetseries.owasp.org/cheatsheets/Key_Management_Cheat_Sheet.html "OWASP Key Management Cheat Sheet"
[6]: https://rustsec.org/ "RustSec Advisory Database and tooling"
