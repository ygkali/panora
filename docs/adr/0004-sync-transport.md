# ADR 0004: Senkron Taşıma Katmanı — Önce LAN (quinn + mDNS), Ayrı Süreç; iroh Ertelendi

- **Durum:** Kabul edildi (SYNC-01 spike sonucu)
- **Tarih:** 2026-09-23
- **İlgili:** ADR 0002 (modüler senkron), ADR 0003 (güvenlik modeli), ROADMAP §5.11, D-9

## Bağlam

ADR 0002 taşıma için **iroh**'u önermişti (bağlayıcı olmadan). ROADMAP SYNC-01 bunun
boyut/bağımlılık etkisini, ayrı bir `panora-sync` ikilisinin ve feature flag'in fizibilitesini
ölçmeyi istiyordu. D-9 ise "önce LAN-only spike" öneriyordu. Bugünkü `panod` tamamen
çevrimdışıdır: systemd biriminde `RestrictAddressFamilies=AF_UNIX`, README'nin ilk
paragrafı "never opens a network connection" der.

## Ölçüm (2026-09-23, WSL2 Ubuntu 24.04, rustc 1.98.1)

Her aday için, depo dışında tek dosyalık bir `tokio` ikilisi yazıldı: iroh için bir `Endpoint`
bağlayıp kimliğini yazdırmak; quinn için kendinden imzalı sertifikayla (rcgen) bir QUIC sunucu
uç noktası açıp `_panora-sync._udp` servisini mDNS ile duyurmak. `release` profili `lto = "thin"`,
`codegen-units = 1`, `strip = true`. Lisans ve güvenlik denetimi deponun kendi `deny.toml`'u ile.

| | quinn 0.11 + mdns-sd 0.21 (+ rustls/ring, rcgen) | iroh 1.2 (`presets::Minimal`, varsayılan özellikler kapalı, `tls-ring`) | iroh 1.2 (varsayılanlar, `presets::N0`) |
|---|---|---|---|
| Kilit dosyasındaki crate | 108 | 332 | 339 |
| Workspace'te **olmayan** yeni crate | **43** | 208 | 213 |
| Soyulmuş ikili | **2,7 MiB** | 11,2 MiB | 13,5 MiB |
| Temiz release derlemesi | 32 sn | 100 sn | 106 sn |
| `cargo deny check licenses` | geçti | **kaldı**: `webpki-roots` (CDLA-Permissive-2.0) | **kaldı**: aynı |
| `cargo deny check advisories` | temiz | temiz | temiz |
| En yüksek bağımlılık MSRV'si | 1.88 | 1.91 | 1.91 |
| Derlenen HTTP yığını | yok | reqwest, hyper, tower (relay istemcisi özellikle kapatılamıyor) | aynı |

Karşılaştırma için: bütün Panora `.deb` paketi (panod + GUI + CLI + eklenti) **4,5 MiB**; workspace
kilit dosyası 351 crate.

Kayda değer iki gözlem daha:

- iroh'un `N0` hazır ayarı, kaynak belgelerine göre uç nokta adresini **n0.computer**'ın DNS
  sunucusuna (`iroh.link`) yayınlar ve Number 0'ın relay sunucularını kullanır. Yani varsayılan
  kurulum üçüncü taraf bir altyapıya IP ve cihaz kimliği sızdırır. Bu, Panora'nın "ağ yok"
  duruşuyla doğrudan çelişir; iroh kullanılacaksa bu ayar kendi relay'imiz veya yalnızca
  doğrudan bağlantıyla değiştirilmelidir.
- Özellikler kapalıyken bile iroh, relay istemcisi ve DNS çözücüsü yüzünden HTTP yığınını,
  netlink izleyicisini ve (derlenmese de kilit dosyasında) Android/Windows/Apple platform
  crate'lerini getiriyor. Kazanç (~%3 daha az crate) önemsiz.

## Karar

1. **Taşıma, ilk sürümde yalnızca LAN: `quinn` (QUIC, rustls + ring) + `mdns-sd` (keşif).**
   Tanımadığımız hiçbir sunucuya bağlanılmaz; relay/NAT delme yoktur. Uzak ağlar arası senkron
   (SYNC-04'ün ikinci yarısı) ayrı bir karar olarak sonraya kalır; o zaman iroh (kendi relay'imizle)
   veya elle yazılmış bir relay yeniden değerlendirilir.
2. **Ayrı süreç: `panora-sync` ikilisi, ayrı systemd birimi, ayrı `.deb`.** `panod` hiçbir ağ
   bağımlılığıyla derlenmez ve `RestrictAddressFamilies=AF_UNIX` kısıtı `panod.service`'te aynen
   kalır. Ağ izni yalnızca `panora-sync.service`'e verilir: `AF_UNIX AF_INET AF_INET6`, ve
   `IPAddressDeny=any` ile birlikte `IPAddressAllow=link-local multicast 10.0.0.0/8
   172.16.0.0/12 192.168.0.0/16 fc00::/7` (systemd'nin özel değerleri yalnızca `localhost`,
   `link-local`, `multicast`; özel ağ aralıkları açıkça yazılmalı). Bu, ROADMAP SYNC-06'daki
   "`RestrictAddressFamilies` yalnızca sync biriminde gevşetilir" maddesiyle birebir örtüşür.
3. **`panora-sync`, `panod` ile mevcut IPC soketi üzerinden konuşur.** Değişiklikleri okumak için
   bugün var olan `Subscribe` olay akışı (STO-08) yeterli. Uzaktan gelen kayıtları yazmak için ise
   yeni bir istek gerekir: `Store` kaydı yerel `device_id`/`lamport` ile damgalıyor ve tombstone
   taşımıyor. Bu, SYNC-03'ün parçası olarak eklenecek: uzak `device_id`, `lamport`, `created_at`
   ve `deleted` alanlarını koruyarak LWW kuralıyla uygulayan bir istek ve "şu Lamport değerinden
   sonraki değişiklikler" sorgusu.
4. **`panora-core`'daki boş `sync` cargo özelliği kaldırılmaz ama kullanılmaz.** Ayrı süreç modeli
   derleme zamanı bayrağını gereksiz kılıyor; paket kurulu değilse kod da yok. `SyncProvider`/`NoopSync`
   daemon'un içinde kalır ve olayları (gerekirse) `Subscribe` akışına ek ayrıntı olarak besler.
5. **İçerik şifrelemesi ADR 0002'deki gibi taşımadan bağımsızdır.** QUIC'in TLS'i yalnızca kanal
   içindir; kayıtlar eşleştirmede türetilen grup anahtarıyla (XChaCha20-Poly1305) ayrıca şifrelenir.
   Cihaz kimliği, eşleştirmede takas edilen kendinden imzalı sertifikanın açık anahtarına
   sabitlenir (rcgen ile üretilir, ayrı bir CA yoktur).

## Sonuçlar

- **Olumlu:** Senkron kurulmadığında Panora'nın ağ yüzeyi yine sıfır. Kurulduğunda ek yük
  ~2,7 MiB ve 43 crate; hepsi mevcut lisans politikası içinde, sıfır danışma kaydı. Sandbox'lı
  çekirdek daemon'a dokunulmuyor.
- **Olumsuz:** Aynı ağda olmayan cihazlar (ör. evdeki masaüstü ile iş yerindeki dizüstü) ilk
  sürümde senkronlanmaz. mDNS bazı kurumsal/misafir ağlarında engellidir; elle `host:port`
  girişi yedek yol olarak gerekecek.
- **Açık iş:** Yeni IPC isteği (SYNC-03), eşleştirme akışı ve anahtar türetimi (SYNC-02),
  `panora-sync` birimi ve paketi, SYNC-06 bağımsız güvenlik incelemesi.

## Yeniden üretim

Ölçüm için kullanılan iki `Cargo.toml` bağımlılık bloğu:

```toml
# quinn + mDNS
quinn = { version = "0.11", default-features = false, features = ["runtime-tokio", "rustls-ring", "log"] }
rustls = { version = "0.23", default-features = false, features = ["ring", "std"] }
rcgen = "0.14"
mdns-sd = "0.21"
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }

# iroh, en hafif hâli
iroh = { version = "1", default-features = false, features = ["tls-ring"] }
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }
```

Sayılar `cargo generate-lockfile`, `cargo build --release` (temiz hedef dizini),
`cargo deny check licenses advisories` (deponun `deny.toml`'u kopyalanarak) ve kilit dosyalarının
workspace'in `Cargo.lock`'u ile farkından alındı.
