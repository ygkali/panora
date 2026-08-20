# Panora güvenlik araştırması — ilk bulgular

## CopyQ

Kaynak: https://github.com/hluk/CopyQ

CopyQ resmi deposu, uygulamanın gelişmiş özellikli bir pano yöneticisi olduğunu ve güvenlik/kalite sekmesi, CI, pre-commit ve Codecov gibi bakım altyapıları kullandığını gösteriyor. Güncel commit notlarında Linux için QtKeychain kullanımı ve hassas içeriklerle ilişkili ignore davranışları görülüyor. Projenin issue geçmişindeki güncel açıklamada parola yöneticilerinden gelen sırların varsayılan olarak yok sayılmaya çalışıldığı, ancak bazı uygulamaların clipboard formatlarını doğru işaretlememesi nedeniyle uygulama adı/format tabanlı ek kuralların gerektiği görülüyor.

Panora için çıkarım: hassas veriyi yalnızca MIME işaretlerine bırakmamak; uygulama adı, format ve kullanıcı tarafından genişletilebilir hariç listesi kullanmak; bunun yanında otomatik test ve bağımlılık/CI kontrollerini sürekli çalıştırmak.

## GPaste

Kaynak: https://github.com/Keruspe/GPaste

GPaste resmi deposundaki güncel kaynak/commit açıklamaları parola öğelerini düz metin geçmişinden hariç tutma, parola değerini maskeleme (`******`) ve daemon/client wire formatında gerçek parola değerini taşımama yaklaşımını gösteriyor. Proje D-Bus tabanlı daemon + client mimarisi kullanıyor. Güncel geliştirme notlarında GNOME Shell extension için upstream GNOME ESLint tooling ve CI entegrasyonu da yer alıyor.

Panora için çıkarım: parolaları yalnızca kaydetmemekle kalmayıp GUI/IPC önizleme katmanında da maskelemek; daemon ve istemci arasında yapılandırılmış veri taşımak; GNOME extension JavaScript'ini upstream ESLint kurallarıyla CI'da denetlemek.

## İlk entegrasyon adayları

1. Preview ve IPC katmanında secret içerik için gerçek payload yerine sabit maske veya isim taşıma.
2. MIME secret flag + uygulama adı + güvenli format sınıflandırmasını tek bir policy gate altında birleştirme.
3. Unix socket için peer credential doğrulaması ve istek boyutu/rate limitleri.
4. SQLite ve BLOB dosyalarında izinlerin her açılışta doğrulanması; WAL/SHM ve geçici dosya sızıntısı testleri.
5. Secret Service API dönüş tipleri ve fallback davranışı için test; master key'in yalnızca keyring'den gelmesi.
6. GNOME extension ESLint/metadata/schema testleri ve bağımlılık/CI güvenlik kontrolleri.
7. RustSec/cargo-deny, SBOM ve reproducible build/checksum yayın süreci.

## Freedesktop Secret Service

Kaynak: https://specifications.freedesktop.org/secret-service/latest-single/

Resmi spesifikasyon Secret Service'i kullanıcının login session'ında çalışan bir serviste secret saklama API'si olarak tanımlar. Model; session, collection, item ve prompt nesnelerinden oluşur. `SearchItems` unlocked ve locked item yollarını ayrı döndürür; `OpenSession` ile secret transfer oturumu açılır; collection kilitli olabilir ve unlock/prompt akışı gerekir. API, uygulamanın session/collection durumlarını doğru ele almasını ve secret transferini API sözleşmesine göre yapmasını gerektirir.

Panora için çıkarım: collection yoksa sessizce yeni anahtar üretip kaydetmek yerine, collection/unlock/prompt durumlarını güvenli ve açık hata durumlarıyla yönetmek; D-Bus dönüş imzalarını doğru test etmek; session kapatmayı ve Secret Service kilit durumunu yönetmek; mümkünse `SearchItems` ve `CreateItem` akışını resmi tuple imzalarıyla integration test etmek.

## OWASP Cryptographic Storage

Kaynak: https://cheatsheetseries.owasp.org/cheatsheets/Cryptographic_Storage_Cheat_Sheet.html

OWASP, kriptografik depolama tasarımının tehd modelinden başlamasını; mümkünse dedicated secret/key management system kullanılmasını; uygun encryption layer'ın tehdide göre seçilmesini; doğrulanmış güncel algoritmaların ve authenticated encryption'ın kullanılmasını; anahtarların veriden ayrılmasını; anahtar yaşam döngüsü, rotation ve secure deletion'ın planlanmasını önerir. OWASP ayrıca şifrelemenin tek başına uygulama güvenliğini sağlamadığını ve anahtar yönetiminin ayrı bir risk alanı olduğunu vurgular.

Panora için çıkarım: mevcut XChaCha20-Poly1305 + Secret Service mimarisini korumak; nonce/ciphertext formatına açık version ve algorithm identifier eklemek; BLOB ve preview için associated data kullanmak; key rotation/migration API'si eklemek; silinen BLOB ve SQLite/WAL artıklarını test etmek; düz metin log/preview sızıntısını denetlemek.

## RustSec

Kaynak: https://rustsec.org/

RustSec resmi sayfası `cargo-audit` ile Cargo.lock içindeki bağımlılıkların advisory veritabanına göre taranmasını, `cargo-deny` ile advisory yanında lisans, kaynak, duplicate version ve izin politikalarının denetlenmesini, `cargo-auditable` ile üretim binary'sine bağımlılık ağacının gömülmesini ve CI/scheduled audit action'larını öneriyor.

Panora için çıkarım: audit'i yalnızca geliştirici makinesine bırakmamak; CI'da `cargo audit`, `cargo deny check`, mümkünse auditable release ve düzenli zamanlanmış tarama kullanmak; sonuçları release checklist'inde raporlamak.

## OWASP Key Management

Kaynak: https://cheatsheetseries.owasp.org/cheatsheets/Key_Management_Cheat_Sheet.html

OWASP anahtar yönetimini yaşam döngüsü olarak ele alıyor: generation, distribution, storage, use, rotation, backup/recovery, compromise recovery ve destruction/zeroization. Uygulamanın kriptografik gereksinimleri ve anahtarların nerede tutulduğu haritalanmalı; anahtarlar veriyle aynı yerde tutulmamalı; anahtar kullanım amacı ayrıştırılmalı; compromise ve rotation senaryoları önceden tasarlanmalıdır.

Panora için çıkarım: keyring item attributes içinde sürüm/amaç/algoritma metadata'sı; master-key rotation için yeni key id ve migration; anahtar kullanımı sonrası zeroize; eski key ile açma ve yeni key ile yeniden şifreleme testi; keyring erişim hatasında fail-closed davranış.
