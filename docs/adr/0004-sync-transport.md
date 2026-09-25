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
   sonraki değişiklikler" sorgusu. *(2026-09-23: eklendi — `SyncApply` ve `SyncChanges`, bkz.
   ROADMAP §11.8.)*
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
- **Açık iş:** Yeni IPC isteği (SYNC-03), eşleştirme akışı ve anahtar türetimi (SYNC-02 — ADR 0005),
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

## Güncelleme (2026-09-25, SYNC-04 uygulaması)

Karar uygulandı (`crates/panora-sync`, `panora-sync` ikilisi, `packaging/panora-sync.service`,
ayrı `panora-sync` `.deb`'i). Uygulamada yukarıdaki kararın dört noktası değişti veya netleşti:

1. **Cihaz kimliği sertifikaya değil TLS oturumuna bağlanıyor (karar 5'in yerine).** Her uç
   nokta her başlangıçta kullanılıp atılan, kendinden imzalı bir sertifika sunuyor. İstemci
   herhangi bir sertifikayı kabul ediyor, ama el sıkışma imzasını yine doğruluyor. Karşıdaki
   cihazın kim olduğu bir adım sonra kanıtlanıyor: iki taraf da bu TLS oturumundan dışa aktarılan
   bir değeri (RFC 5705/8446 exporter) kendi rolüyle birlikte Ed25519 cihaz kimliğiyle imzalıyor
   (`transport::authenticate`).
   - Araya giren biri iki ayrı TLS oturumu kurar, yani iki farklı dışa aktarım değeri görür. Bu
     yüzden imzaları öbür oturuma taşıyamaz. Rol etiketi de bir imzanın imzalayana geri
     yansıtılmasını önler.
   - Böylece X.509 hiç ayrıştırılmıyor. Sertifikadan anahtar çıkaran basit bir ayrıştırıcı,
     uzantıya gömülmüş sahte bir anahtarla kandırılabilirdi.
   - ALPN iki protokolü ayırıyor: `panora-sync/1` ve `panora-pair/1`. Eşleştirme kendi
     kimlik doğrulamasını yapıyor (ADR 0005).
2. **Hangi adreslerle konuşulacağı programın içinde uygulanıyor (karar 2'deki `IPAddressAllow`
   yerine).** systemd 255'in kullanıcı yöneticisi `IPAddressDeny=`/`IPAddressAllow=`
   ayarlarını kullanıcı birimlerinde uygulamıyor. WSL2 Ubuntu 24.04'te `systemd-run --user -p
   IPAddressDeny=any` altında 1.1.1.1:80'e bağlantı yine kuruldu. `RestrictAddressFamilies=`
   ise uygulanıyor (aynı denemede `AF_UNIX` ile bağlantı reddedildi).
   - Bu yüzden birimde yalnızca `RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6 AF_NETLINK`
     var. `AF_NETLINK`, mDNS ve davetler için ağ arayüzlerini listeliyor.
   - Adres sınırı `transport::is_allowed_peer` ile iki yönde de her bağlantıya uygulanıyor:
     loopback, RFC 1918, link-local ve IPv6 unique-local. Genel bir adresten gelen bağlantı el
     sıkışmadan önce reddediliyor. mDNS'in ya da yapılandırmanın verdiği genel bir adres hiç
     aranmıyor.
   - `panod.service`'in `RestrictAddressFamilies=AF_UNIX` kısıtı gerçekten uygulanıyor. Bu da
     aynı denemeyle doğrulandı.
3. **Silme kayıtları senkron açıkken tutuluyor (SYNC-03'ün açık maddesi 1).** `sync.enabled`
   açıkken kullanıcının sildiği bir kaydın satırı, yükleri geri alma süresinden sonra silinmiş
   olarak, `sync.tombstone_days` gün (varsayılan 30) kalıyor. Böylece çevrimdışı kalmış bir
   cihaz silmeyi öğreniyor, kaydı geri göndermiyor.
   - Saklama kurallarının çıkardığı kayıtlar (`deleted_at = 0`) akışa hiç girmiyor. Bunlar bu
     cihazın kendi temizliği; başka cihazların tekrar etmesi gereken bir silme değil.
   - Senkron açıkken geri alma, satır dursa da 30 saniyeden sonra reddediliyor.
4. **mDNS'te grup kimliği görünmüyor.** `_panora-sync._udp` yalnızca grup kimliğinden ve
   saatten türetilen bir etiket duyuruyor; örnek adı da her çalıştırmada rastgele. Yabancı biri
   hangi grubu gördüğünü öğrenemiyor, iki görüşün aynı grup olduğunu da ancak bir saat içinde
   anlayabiliyor.
   - `_panora-pair._udp` yalnızca bir eşleştirme penceresi açıkken duyuruluyor. Cihaz adını ve
     kimlik anahtarından türetilen kısa bir etiketi taşıyor; davetli cihaz doğru davet edeni
     bu etiketle buluyor.

Oturum protokolü:
- İki taraf da `Hello` ile başlar. Bu mesaj cihaz kimliğini ve tutulan bütün roster'ları taşır.
- Bir cihaz, karşı tarafı üye olarak tanımıyorsa kendi `Hello`'sunu göndermez. Önce karşı
  tarafın, onu üye yapan bir zincir göstermesini bekler. Beklerken karşı taraftan en fazla
  512 KiB'lık tek bir çerçeve okunur, ardından 15 saniyelik zaman aşımı gelir.
- Kayıtlar grup anahtarıyla mühürlenir. Şifreli metinler çerçevenin ikili ekleri olarak gider,
  yani base64 şişmesi olmaz.
- Her eşin imleci yalnızca `Applied` onayıyla ilerler. `Retry` imleci yerinde bırakır; alıcıda
  anahtar yoksa ya da daemon kilitli veya kapalıysa bu yol kullanılır.
- Karşı cihazın yaptığı durumlar ona geri gönderilmez.
- Aynı cihazla iki oturum açılırsa her iki tarafta da düşük kimliğin açtığı oturum kalır.

Bağımlılıklar kilit dosyasına 43 yeni crate ekledi (quinn, rustls, rcgen, mdns-sd ve
bağımlılıkları). Bunlar yalnızca `panora-sync` ikilisine giriyor; `cargo deny` lisans ve danışma
denetimleri temiz.

Boyut:
- Soyulmuş `panora-sync` ikilisi 11,3 MB, `panod` 11,7 MB. Yukarıdaki 2,7 MiB'lık ölçüm
  yalnızca QUIC + mDNS içindi; gerçek ikiliye Secret Service/D-Bus istemcisi (oo7, zbus), tokio
  ve CLI da giriyor.
- `panora-sync` `.deb`'i ayrı bir paket, ana paketin boyutunu değiştirmiyor.

Doğrulama:
- Gerçek `panod` sunucularıyla süreç içi uçtan uca testler: iki ve üç cihaz; davetle ve kodla
  eşleştirme; kayıt, silme ve sabitlemenin iki yönde gitmesi; çıkarma; yabancının hiçbir şey
  öğrenmemesi.
- Gerçek ikililerle duman testi: aynı makinede üç ayrı "cihaz", ayrı XDG dizinleri, ayrı Xvfb
  ekranları ve paylaşılan gnome-keyring. Davet bağlantısı, mDNS ile bulunan kodlu eşleştirme,
  iki yönlü senkron ve çıkarma bu testte çalıştı.
- Birimin sandbox ayarlarıyla (`systemd-run --user`) başlatma: anahtarlık, bağlanma, mDNS ve
  `status` çalışıyor.

### İç inceleme ve sonrası (2026-09-25)

Ağ katmanı, commit'ten önce ayrı bir ajan tarafından saldırgan gözüyle incelendi: 3 yüksek,
3 orta, 5 düşük bulgu. Hepsi kapatıldı; her birinin regresyon testi var.

- **Aktarılan kayıt atlanıyordu (yüksek, doğruluk).** Değişiklik akışı Lamport değerine göre
  sıralanıyordu, ama başka bir cihazdan uygulanan satır o cihazın düşük değerini koruyor. Bu
  yüzden üçüncü bir cihazın imleci o değeri çoktan geçtiyse satır ona hiç gitmiyordu.
  - Düzeltme: veritabanı şeması 5. Yeni `change_seq` sütunu ve `meta` içinde tutulan, geri
    gitmeyen bir sayaç var. Satır eklendiğinde ve `lamport`/`device_id`/`deleted`/`pinned`
    değiştiğinde SQLite tetikleyicileriyle bir sonraki değeri alıyor.
  - Akış ve `SyncCursor` artık bu sayaca göre ilerliyor. Lamport değerleri yalnızca LWW için
    kullanılıyor.
  - Eski dosyalar yedeklenip taşınıyor; mevcut satırlar id sırasıyla numaralanıyor.
- **Roster kanıtlanmamış oturuma gidiyordu (yüksek, sızıntı).** Oturum, `Hello` doğrulanmadan
  kaydediliyordu. Bu arada yapılan bir roster yayını, 15 saniyelik bekleme süresindeki yabancıya
  da gidiyordu.
  - Düzeltme: `Rosters` ve `KeyShare` komutları yalnızca `Hello`'su kabul edilmiş oturuma
    iletiliyor.
  - Gelen `KeyRequest`, `KeyShare` ve `Records` her seferinde üyelik denetiminden geçiyor.
  - `Hello`'daki `device_id` roster'dakiyle eşleşmek zorunda.
- **Kimliği doğrulanmamış bağlantılar belleği şişirebiliyordu (yüksek, DoS).** Düzeltme:
  - Bağlantı başına tek iki yönlü akışa izin var, tek yönlü akış yok.
  - Akış penceresi 4 MiB, bağlantı penceresi 8 MiB.
  - Aynı anda en fazla 64 bağlantı, bir adresten 4.
  - Akış açma ve kimlik iletisi için 10 saniye zaman aşımı.
  - Çerçeve okuyucu, bildirilen uzunluğu baştan ayırmıyor; gelen veri kadar büyüyor.
- **Eşleştirme penceresi tüketilebiliyordu (orta).** Düzeltme:
  - Deneme hakkı ve pencere kilidi ancak geçerli bir `Commit` geldikten sonra kullanılıyor.
  - Her adımın 30 saniyelik zaman aşımı var.
  - Pencerelerin bir numarası var: kapatma yalnızca kendi penceresini kapatıyor ve süren bir
    eşleştirmeyi, kimseyi kabul etmeden iptal ediyor.
  - Yeni cihaz grubun bir kopyasına ekleniyor; kopya ancak karşılama mesajı gittikten sonra
    kalıcı oluyor.
- **`leave` kimliği yalnızca dosyada değiştiriyordu (orta).** Süreç eski kimlikle devam ediyor,
  sonraki başlangıçta durum dosyası reddediliyordu. Düzeltme: kimlik bir kilidin arkasında
  tutuluyor ve `leave` onu da değiştiriyor.
- **Tek bir büyük kayıt akışı kalıcı olarak tıkıyordu (orta).** Düzeltme: `panod` 64 MiB'tan
  büyük kayıtları akışa hiç koymuyor. Mühürlü bir sayfa böylece alıcının çerçeve sınırına
  sığıyor.
- **Düşük önemdekiler:**
  - Alıcı gizli moddayken veya kilitliyken kayıtlar `Retry` ile geri gönderiliyor. Önceden
    "yok sayıldı" onayı alıp kayboluyorlardı.
  - Bir adresi bir kez yabancı yanıtladıysa o adres yine deneniyor.
  - mDNS önbelleği 256 kayıtla sınırlı.

**Bilinen sınır:**
- Dinleyen cihaz, kimliği doğrulanmış her bağlantıya uzun ömürlü Ed25519 kimlik anahtarını
  gösteriyor; kimlik kanıtının kendisi bu. Yerel ağdaki biri bu yüzden bir cihazı zaman içinde
  tanıyabilir. Saatlik mDNS etiketi bu bağlantıyı yalnızca pasif gözlemciden saklıyor.
- Grubun kimliği hiç değişmediği için, çıkarılmış bir cihaz grup etiketini hesaplamayı
  sürdürebilir.
- Anahtar gösterilmeden önce bir gizli el sıkışma eklemek relay aşamasına bırakıldı.

### Birleştirme öncesi son inceleme (2026-09-25)

`main`'e birleştirmeden önce bütün yığın (panod'un senkron IPC'si, `panora-sync`, GUI) aynı ağdaki
saldırgan gözüyle bir kez daha incelendi. Gizlilik ve kimlik doğrulamayı bozan bulgu yok. Kapatılanlar:

- **Lamport zehirlemesi (orta).** Bir üye tavanın hemen altında bir değer gönderince alıcının bir
  sonraki değeri tavana değiyor, öteki cihazlar onu ve sonrasını sessizce reddediyordu; senkron o
  cihaz için kalıcı olarak duruyordu. Artık iki sınır var (`panora_core::sync`): 2^53'e kadar kabul,
  ama saat yalnızca 2^52'nin altını izliyor (satırlardan okunan en büyük değer dahil).
- **Uzak zaman damgaları (orta).** `created_at`/`last_seen_at` olduğu gibi alınıyordu; gelecek tarihli
  bir kayıt `max_age_days`'ten kaçıyor ve listenin başında kalıyordu. Artık `[0, şimdi + 5 dk]`.
- **PRIMARY (düşük–orta).** `record_primary` kapalı cihaz, seçim panosu kayıtlarını başka cihazdan
  alıyordu. Artık almıyor.
- **Davet penceresini yabancılar tüketebiliyordu (düşük).** `Commit` artık davet sırrıyla bir kanıt
  taşıyor; davet eden, deneme hakkı düşmeden ve kilidi almadan önce kanıtı ve modu denetliyor. Kod
  modunda bu mümkün değil (kod karşılaştırılmadan kimin kim olduğu bilinemez); 3 deneme sınırı kalıyor
  ve `docs/SYNC.md` bunu söylüyor.
- **Grup kimliği kimliği doğrulanmamış tarafa gidiyordu (düşük).** `Offer` artık `group_id` taşımıyor
  (transkriptten de çıktı); ona yalnızca imzalı, mühürlü karşılama iletisiyle ulaşılıyor. Böylece bir
  yabancı saatlik mDNS etiketini hesaplayamıyor.
- **mDNS önbelleği (düşük).** Yalnızca bu grubun etiketini (bir önceki, bu ve bir sonraki saat) taşıyan
  duyurular tutuluyor; yabancılar önbelleği doldurup üyeleri gizleyemiyor.
- **Adres kuralı (düşük).** mDNS'ten ve kendi adreslerinden loopback çıkarıldı; izin verilmeyen
  adresten gelen bağlantı artık yanıtsız düşürülüyor (`ignore`), tarayıcıya QUIC olduğunu söylemiyor.
  "Özel adres" ile "aynı oda" aynı şey değil: yönlendirilen özel ağlardan da bağlanılabiliyor
  (üyelik kanıtı yine şart); `docs/SYNC.md` bunu açıkça söylüyor.
- **Bellek (düşük).** Çerçeve sınırı 192 MiB'tan 64 MiB'a, akıştaki tek kayıt sınırı 64'ten 48 MiB'a
  indi; birime `MemoryMax=768M` eklendi.

Doğrulanmadı: birimin sandbox'ı gerçek bir Ubuntu 24.04 masaüstünde (AppArmor'un ayrıcalıksız kullanıcı
ad alanı kısıtıyla) denenmedi; `panod.service` ile aynı durum.

### Farklı ağlar: ertelendi (2026-09-25)

Sahibinin kararıyla 2.0 yalnızca aynı ağdaki cihazları eşitler. Relay, VPN adresleri ya da iroh
gibi uzak ağ yolları ROADMAP'te SYNC-07 ("Gelecek") altında duruyor; seçenekler ve her birinin
bedeli orada. Bu ADR'deki adres kuralı (yalnızca loopback, RFC 1918, link-local, ULA) o zamana
kadar değişmez.
