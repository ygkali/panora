# ADR 0003: Güvenlik Modeli — Şifreleme, Gizlilik Bayrakları, Sır Yönetimi

- **Durum:** Kabul edildi
- **Tarih:** 2026-08-18

## Bağlam

Pano yöneticileri doğası gereği en hassas kullanıcı verilerine (parolalar, kimlik bilgileri, özel mesajlar) dokunur. Projenin güvenilirliği, bu verilerin nasıl işlendiğine bağlıdır.

## Karar

### 1. Diskte şifreleme

Tüm pano kayıtları (SQLite içeriği ve BLOB dosyaları) **XChaCha20-Poly1305** ile şifrelenir. Kayıt başına 192-bit rastgele nonce üretilir; nonce tekrarı riski yoktur. Ana şifreleme anahtarı (32 bayt) ilk çalıştırmada üretilir ve **Secret Service API** (GNOME Keyring / KWallet) üzerinden saklanır; diske asla düz yazılmaz. Anahtar bellekte `zeroize` ile kullanım sonrası silinir.

### 2. Gizlilik bayrakları (parola yöneticisi koruması)

Aşağıdaki MIME ipuçlarını taşıyan içerik **hiç okunmadan** atlanır: `x-kde-passwordManagerHint` (değeri `secret` ise), `org.nspasteboard.ConcealedType`, `application/x-nspasteboard-concealed-type`.

**Kritik sıralama (yarış koşulu önlemi):** Pano değişim bildirimi geldiğinde önce yalnızca TARGETS/MIME listesi okunur; gizlilik bayrağı varsa içerik asla istenmez. Bu, CopyQ'nun KeePassXC ile yaşadığı #2802 hatasının doğrudan önlemesidir.

### 3. Uygulama hariç tutma

KeePassXC, Bitwarden ve 1Password varsayılan hariç listesindedir; kullanıcı ayarlar penceresinden listeyi genişletebilir.

### 4. Özel mod (private mode)

Geçmiş kaydı geçici olarak duraklatılabilir; duraklatma sırasında pano değişimleri izlenmez ve saklanmaz.

### 5. Ağ yüzeyi

v1.0 **tamamen çevrimdışıdır**: hiçbir ağ bağlantısı açmaz, telemetri içermez. Senkron modülü (ayrı paket) gelene kadar bu böyle kalır.

### 6. Kod tabanı kuralları

Mümkün olan tüm crate'lerde `#![forbid(unsafe_code)]` uygulanır. CI'da `cargo audit` (RustSec) ve `cargo deny` (lisans + yasaklar) zorunludur. Sır içeren tüm türler `Zeroize`/`ZeroizeOnDrop` türetir.

## Gerekçe

XChaCha20-Poly1305, nonce-misuse'a karşı AES-GCM'den dayanıklıdır ve donanım AES desteği olmayan cihazlarda da hızlıdır. Secret Service, GNOME ve KDE'de standart anahtarlık arayüzüdür; kullanıcı parolasıyla korunur ve düz dosyadan güvenlidir. TARGETS-önce sıralaması zorunludur çünkü bayrak kontrolü içerik okunmadan yapılmalıdır; aksi halde parola belleğe veya diske düşer.

## Sonuçlar

**Olumlu:** Disk ele geçirilse bile geçmiş okunamaz; parola yöneticisi verisi hiç kaydedilmez; denetlenebilir küçük bir saldırı yüzeyi elde edilir.

**Olumsuz:** Anahtarlık kilitliyse (oturum açılmamışsa) daemon ilk açılışta anahtara erişemez; bu durumda kullanıcıya açık bir hata mesajı gösterilir.

**Sınırlama:** Bellek dökümü (core dump) veya takas (swap) üzerinden anahtar sızıntısı bu kapsamda ele alınmaz; `mlock` değerlendirmesi ileri sürüme bırakılmıştır.
