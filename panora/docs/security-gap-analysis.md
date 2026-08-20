# Panora güvenlik boşluk analizi

**Kapsam:** CopyQ ve GPaste'nin resmi açık kaynak kaynakları; freedesktop Secret Service API; OWASP Cryptographic Storage ve Key Management; RustSec tooling.

| Kontrol alanı | Açık kaynak/pratik bulgusu | Panora mevcut durumu | Karar |
|---|---|---|---|
| Hassas pano algılama | CopyQ parola yöneticisi içeriklerini ignore etmeye çalışıyor; GPaste parola öğelerini düz metin geçmişinden çıkarıyor ve değerleri maskeliyor | MIME flag ve uygulama hariç listesi var; ancak IPC/GUI modelinde hassasiyet sınıfı yok | **Entegre:** privacy verdict'i storage/UI sınırına taşı, secret preview asla dönmesin |
| Secret Service | Resmi API session/collection/item/locked/prompt akışını tanımlıyor | Secret Service kullanılıyor; collection yokluğu fallback ile yeni item oluşturuyor; dönüş imza testi sınırlı | **Entegre:** resmi dönüş tipleri, collection/prompt hata ayrımı, fail-closed ve integration test |
| Şifreli depolama | OWASP AEAD, anahtarların veriden ayrılması, yaşam döngüsü ve destruction ister | XChaCha20-Poly1305 ve keyring var; format sürümü/AAD/rotation API'si yok | **Entegre:** versioned envelope, AAD ve master-key rotation için migration primitive |
| IPC erişim kontrolü | Daemon/client ayrımı; güvenli açık kaynak örneklerde sınırlı wire yüzeyi | 0600 socket var; peer UID, frame boyutu ve client flood limiti yok | **Entegre:** SO_PEERCRED UID kontrolü, 64 KiB frame sınırı, bağlantı başına istek limiti |
| Dosya izinleri | Yerel history'nin kullanıcı hesabına özel olması gerekir | DB 0600; BLOB/root/WAL/tmp izinleri her yerde zorlanmıyor | **Entegre:** root, shard, blob ve tmp dosyaları 0700/0600; symlink/path validation |
| Supply chain | RustSec `cargo-audit`, cargo-deny, cargo-auditable ve CI/scheduled tarama öneriyor | CI audit/deny job'ları var; yerel script bunları çalıştırmıyor; audit sandbox ağında kurulamadı | **Entegre:** CI fail-closed, SBOM/auditable release opsiyonları ve yerel araç yoksa açık durum |
| GNOME extension | GPaste upstream GNOME ESLint ve CI kullanıyor | Panora JS syntax kontrolü var; upstream ESLint yok | **Entegre:** extension metadata/schema/ESLint kontrolü veya ortam yoksa statik fallback |
| Log/diagnostic sızıntısı | Secret değerleri wire/log katmanına taşınmamalı | Hata mesajları genel olarak güvenli; source app/preview log kapsamı explicit değil | **Entegre:** secret içerik loglamayı test et, `Debug` redaction ve bounded errors |

## Öncelik sırası

P0 olarak secret preview/UI redaction, IPC frame/peer kontrolü, dosya izinleri ve Secret Service fail-closed davranışı ele alınmalıdır. P1 olarak versioned crypto envelope, key rotation primitive, secure cleanup, extension lint ve reproducible/supply-chain kontrolleri eklenmelidir. P2 olarak gerçek GNOME/Wayland compositor matrix'i, keyring lock/unlock integration testleri ve external audit hazırlanmalıdır.

Bu tablo bir sertifikasyon iddiası değildir. OWASP ve RustSec rehberleri tasarım/doğrulama girdisi olarak kullanılmıştır; Panora'nın güvenlik seviyesi ancak tehdit modeline göre testler ve bağımsız inceleme ile değerlendirilebilir.

## Kaynaklar

[1]: https://github.com/hluk/CopyQ "CopyQ resmi GitHub deposu"
[2]: https://github.com/Keruspe/GPaste "GPaste resmi GitHub deposu"
[3]: https://specifications.freedesktop.org/secret-service/latest-single/ "Freedesktop Secret Service API"
[4]: https://cheatsheetseries.owasp.org/cheatsheets/Cryptographic_Storage_Cheat_Sheet.html "OWASP Cryptographic Storage Cheat Sheet"
[5]: https://cheatsheetseries.owasp.org/cheatsheets/Key_Management_Cheat_Sheet.html "OWASP Key Management Cheat Sheet"
[6]: https://rustsec.org/ "RustSec Advisory Database and tooling"
