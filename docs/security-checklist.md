# Panora güvenlik kontrol listesi

| Kontrol | Durum | Kanıt / not |
|---|---:|---|
| Diskte BLOB şifreleme | Geçti | XChaCha20-Poly1305; `storage::blob` testleri |
| Preview şifreleme | Geçti | SQLite `preview` sütunu nonce-prefixed ciphertext hex |
| AEAD bütünlük kontrolü | Geçti | Tamper/wrong-key/truncated testleri |
| Anahtarın diske düz yazılmaması | Tasarım geçerli | Secret Service D-Bus; anahtar `MasterKey` ile zeroize |
| Parola MIME bayrakları | Geçti | TARGETS/MIME listesi payload okunmadan değerlendirilir |
| Varsayılan parola uygulaması hariçleri | Geçti | KeePassXC, Bitwarden, 1Password |
| Özel mod | Geçti | Privacy engine + daemon senaryosu |
| MIME boyut sınırı | Geçti | 10 MiB varsayılan, daemon testi |
| IPC socket izinleri | Geçti | Unix socket ve device-id 0600 |
| v1 ağ yüzeyi | Geçti | `SyncProvider` yalnızca `NoopSync`; workspace ağ bağımlılığı içermez (`scripts/security-check.sh` `cargo tree` ile doğrular) |
| IPC istemci yanıt sınırı | Geçti | 64 MiB yanıt üst sınırı, base64 payload; GUI/CLI `panora_core::ipc::client` |
| Yakalama yarışı | Geçti | Payload okunduktan sonra TARGETS yeniden okunur; sahip değiştiyse kayıt atılır |
| Kalıcılık ve parola temizleme | Geçti | X11'de yalnızca son kaydedilen içerik yeniden sunulur; Wayland'de yeniden sunum yok |
| Unsafe Rust | Geçti | Uygulama crate'leri `forbid(unsafe_code)`; bağımlılıklar kapsam dışı |
| RustSec audit | Ortam engeli | `cargo-audit` kurulumu wasmparser indirme hız sınırına takıldı; CI workflow RustSec action içerir |
| Lisans/bans | CI'da tanımlı | `deny.toml` + cargo-deny workflow; yerel cargo-deny kurulumu ayrıca yapılmalıdır |

> **Güvenlik kapsamı:** Panora v1 clipboard geçmişini yerel kullanıcı hesabında korur. Kernel, swap/core dump, kötü amaçlı GNOME extension veya zaten ele geçirilmiş kullanıcı oturumu bu sürümün tehdit modelinin dışındadır.

## 2026 güvenlik standardı entegrasyonu

| Yeni kontrol | Durum | Uygulama |
|---|---:|---|
| Secret Service `SearchItems` uyumluluğu | Geçti | Secret Service 0.2 tuple yanıtı ve eski GNOME Keyring `ao` yanıtı desteklenir; locked item fail-closed |
| Secret Service prompt fail-closed | Geçti | `CreateItem` interactive prompt döndürürse daemon hata verir; prompt otomatik onaylanmaz |
| Versioned crypto envelope | Geçti | `[version][nonce][ciphertext+tag]`, eski legacy envelope yalnızca migration read için kabul edilir |
| AEAD associated data | Geçti | BLOB ciphertext content hash'e, preview ciphertext entry content hash'e bağlanır |
| BLOB path traversal | Geçti | Hash tam 64 lowercase hex doğrulanır; geçersiz path reddedilir |
| Atomic private blob write | Geçti | `create_new`, 0600 temporary file, `sync_all`, atomic rename |
| Local data permissions | Geçti | data root/blob shard 0700, DB/blob/device/socket 0600 |
| IPC peer identity | Geçti | Unix `SO_PEERCRED` UID socket sahibiyle karşılaştırılır |
| GNOME köprüsü çağıran kimliği | Geçti | `Push`/`PushMany` yalnızca `org.gnome.Shell` adının o anki sahibinden kabul edilir; oturum veriyolundaki diğer süreçler (ör. yalnızca `--socket=session-bus` izinli bir Flatpak) `AccessDenied` alır. Sahip adı önbelleklenir, gnome-shell yeniden başlarsa tekrar sorulur |
| IPC resource limits | Geçti | 64 KiB frame, bağlantı başına 256 istek, query limit 500 |
| Untrusted metadata limits | Geçti | 128 MIME, MIME başına 256 byte, source app 256 Unicode scalar sınırı |
| GNOME extension network surface | Geçti | Static metadata, network primitive/URL yasağı, session D-Bus sınırı |
| FTS preview binding | Geçti | Preview AAD content hash'e bağlı; legacy read fallback yalnızca eski kayıtlar için |
| RustSec/cargo-deny CI | CI zorunlu | Audit, deny ve haftalık schedule job'ları; yerel kurulum yoksa açıkça raporlanır |

**Kaynaklar:** Freedesktop Secret Service API, OWASP Cryptographic Storage ve Key Management Cheat Sheets, RustSec advisory tooling, CopyQ ve GPaste resmi depoları. Ayrıntılı araştırma ve boşluk analizi `docs/security-research-notes.md` ve `docs/security-gap-analysis.md` içindedir.
