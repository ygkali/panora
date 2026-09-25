# ADR 0005: Eşleştirme, Cihaz Listesi ve Grup Anahtarı (`panora-pair/1`)

- **Durum:** Kabul edildi (SYNC-02); SYNC-06 bağımsız incelemesinden geçmeden varsayılan kapalı
- **Tarih:** 2026-09-25
- **İlgili:** ADR 0002 (modüler senkron), ADR 0003 (güvenlik modeli), ADR 0004 (LAN taşıması),
  ROADMAP §5.11 SYNC-02, §11.9

## Bağlam

ROADMAP SYNC-02 üç şey istiyor: **eşleştirme** (QR + kısa doğrulama kodu), **cihaz listesi** ve
**grup anahtarı**. ADR 0002 bunun ana hatlarını çizmişti ("QR kod + 6 haneli doğrulama parmak
izi", "XChaCha20-Poly1305, eşleştirmede türetilen grup anahtarı"). ADR 0004 de cihaz kimliğinin
kendinden imzalı QUIC sertifikasının açık anahtarına sabitleneceğini söylüyordu. Taşıma katmanı
(SYNC-04) henüz yok. Bu nedenle bu karar ağdan bağımsız olarak uygulandı: `crates/panora-sync`
kütüphanesi soket açmıyor ve `panod` bu crate'e bağımlı değil.

Hedef kullanıcı tek bir kişi ve onun 2–5 cihazı (masaüstü, dizüstü, ileride telefon). Cihazların
çoğunda kamera yok. Bu yüzden masaüstünden masaüstüne eşleştirme QR taramadan da çalışmalı.

### Tehdit modeli

| Saldırgan | Beklenen sonuç |
|---|---|
| Aynı LAN'da pasif dinleyici | Hiçbir şey okuyamaz (anahtarlar geçici X25519'dan türetilir, yükler AEAD ile mühürlenir). |
| Aynı LAN'da aktif saldırgan (ARP/mDNS sahteciliği, araya girme) | Davet modunda eşleşemez. Kod modunda deneme başına 10⁻⁶ şansı var; davet eden taraf bir pencerede, yarıda bırakılanlar dahil, en fazla 3 oturum başlatır. |
| Davet bağlantısını/QR'ı gören biri | Gruba katılabilir. Bu yüzden davet yalnızca davet edenin ekranında gösterilir, 10 dakika yaşar ve bir kez kullanılır. |
| Çalınan veya kaybolan, kilidi açık cihaz | Başka bir cihazdan çıkarılır, grup anahtarı yenilenir, bundan sonra paylaşılanı okuyamaz ve eski anahtarla veri enjekte edemez. Çıkarılmayı görmüş hiçbir cihazda, eski bir ebeveyn üzerine imzaladığı rakip roster'larla geri dönemez (bkz. çatal kuralı). Çıkarılmayı henüz görmemiş bir cihaza ondan önce ulaşırsa orada kalıcı bir bölünme yaratabilir (§5, madde 2). |
| Grubun aktif, ele geçirilmiş bir üyesi | Kapsam dışı: tüm üyeler eşit ve tam güvenilir. Çıkarılana kadar her şeyi yapabilir. |
| Cihazdaki kötü amaçlı yazılım | Kapsam dışı (ADR 0003 ile aynı: aynı kullanıcı oturumundaki süreç zaten Secret Service'e erişir). |

## Karar

### 1. Cihaz kimliği: Ed25519, `ring`

Her cihaz bir Ed25519 anahtar çifti üretir (`DeviceIdentity`, PKCS#8 v2). Açık anahtar roster'da
listelenir ve diğer cihazlar onu sabitler. SYNC-04'te aynı PKCS#8 belgesi rcgen'e verilip QUIC
sertifikası yapılacak, böylece TLS eşi sertifika anahtarı roster'la karşılaştırılarak doğrulanır.
Kullanıcıya gösterilen parmak izi: açık anahtarın BLAKE3 türetmesinin 80 biti, `3f2a-91c0-…`
biçiminde. Bu yalnızca aynı adlı iki cihazı ayırt etmek için var; eşleştirmenin güvenliği ona
dayanmıyor.

### 2. Protokol: `panora-pair/1`

Durum makinesi iki taraflı ve G/Ç'siz (`Joiner`, `Inviter`). `wire` modülü bunları herhangi bir bayt
akışında çalıştırıyor (4 bayt uzunluk + JSON, en fazla 64 KiB).

```text
Katılan (J)                                   Davet eden (I, bir üye)
  Commit { protokol, mod, H(eJ) }      ─────►
                                       ◄─────  Offer { eI, kimlik_I, grup_id }
  Reveal { eJ }                        ─────►  H(eJ) denetlenir
       ikisi de: paylaşılan = X25519(eI, eJ), th = H(transkript)
       anahtarlar = KDF(paylaşılan ‖ davet sırrı ya da sıfırlar ‖ th)
       kod modunda: iki ekranda aynı 6 hane (ör. "042 917")
  Join (mühürlü) { kimlik_J, device_id, ad, imza_J(th) } ─────►
                                       ◄─────  Welcome (mühürlü) { imza_I(th), 2 roster, grup anahtarı }
```

- **Davet modu (QR/bağlantı):** `panora-pair:1?id=<I'nın açık anahtarı>&s=<256 bit sır>&exp=<unix>&addr=<ip:port>`.
  Katılan cihaz, Offer'daki kimliği davetteki anahtarla karşılaştırır. Sır anahtar türetimine
  karışır: daveti görmemiş biri, I'nın açabileceği bir `Join` mühürleyemez. Sır sızmış olsa bile
  `Welcome`, I'nın kimlik anahtarıyla imzalanmak zorunda olduğu için saldırgan I'yı taklit
  edemez (testte gösterildi).
- **Kod modu (kamerasız masaüstü ↔ masaüstü):** Önceden paylaşılan bir şey yok. İki cihaz da
  anahtarlardan türetilmiş 6 haneli bir kod gösterir ve kullanıcılar iki ekranda da onaylar.
  Katılan, geçici anahtarına **Offer'ı görmeden önce** bağlanır (commitment). Böylece araya giren
  biri her iki taraftaki anahtarını, kodların ne çıkacağını bilmeden seçmek zorunda kalır ve
  eşleşme olasılığı 10⁻⁶ olur. Ancak katılan rolündeki saldırgan, davet edenin kodunu Offer'ı
  aldığı anda (kendi anahtarını açmadan önce) hesaplayabilir ve kod işine yaramazsa oturumu
  sessizce bırakıp yeniden deneyebilir. Bu yüzden `PairingWindow` **başlayan her oturumu**,
  bitsin bitmesin, bir deneme sayar. `Inviter` pencereyi oturum boyunca `&mut` ile ödünç alır,
  yani aynı anda tek oturum çalışır. Kod penceresi 5 dakika ve 3 oturumla sınırlı; başarılı
  eşleştirme pencereyi kapatır. Toplam olasılık ≤ 3·10⁻⁶. Davet penceresi 10 oturuma izin verir
  (256 bit sır tahmini zaten engelliyor; sınır yalnızca yabancının yaratabileceği işi sınırlar)
  ve bir kez başarılı olunca kapanır. `approve`, pencerenin süresinin dolmadığını yeniden
  denetler.
- **Mod transkriptin parçası:** Her taraf yalnızca kendi kullanıcısının seçtiği modu kabul eder
  (`ModeMismatch` → `Abort{wrong_mode}`). Bu yüzden bir mod diğerine düşürülemez.
- **Türetmeler BLAKE3 ile:** `derive_key` bağlam dizgeleri (`"panora-pair/1 transcript"`,
  `"… session key"`, `"… commitment"`) ve yön başına `keyed_hash` (`"joiner to inviter"`,
  `"inviter to joiner"`, `"short authentication string"`). Mühürleme `panora_core`'un
  XChaCha20-Poly1305 zarfıyla yapılır; AAD = `th`.
- **`ring` düşük dereceli nokta (low-order point) reddini yapar:** Tamamı sıfır bir paylaşılan
  sır üreten eş anahtarı reddedilir (testte gösterildi).
- **Her iki taraf kimlik anahtarıyla `th`'yi imzalar:** J, sertifikası sabitlenecek anahtara
  sahip olduğunu kanıtlar; I, davetteki (veya kodla doğrulanan) kimliğin gerçekten kendisi
  olduğunu kanıtlar.
- **Hata oturumu bitirir:** Durum `Option::take` ile tüketilir; hatadan sonraki her çağrı
  "sıra dışı" hatası verir. Sürücüler karşı tarafa nedenini (`rejected`, `wrong_mode`, `closed`,
  `refused`, `failed`) bildirir.

### 3. Cihaz listesi: imzalı roster zinciri

Her üyelik değişikliği yeni bir **roster** üretir: tam üye listesi (kimlik, `panod` device_id, ad,
eklenme zamanı), bir öncekinin epoch'u + 1, bir öncekinin hash'i ve **bir önceki roster'da üye
olan** bir cihazın Ed25519 imzası. Epoch 2³²−1 ile sınırlıdır; karşı tarafın gönderdiği bir değer
toplamada taşma yaratamaz. Kurallar (`check_successor`):

- Üyeler kimliğe göre sıralı ve tekil, device_id'ler tekil, en fazla 16 cihaz.
- Ad 1–64 karakter, baş/son boşluk yok, denetim karakteri yok, **yön değiştirme karakteri
  (U+202E vb.) yok**. Yoksa bir cihaz adı başka bir ad gibi görünebilirdi.
- Var olan bir üyenin bilgileri değiştirilemez.
- **Cihaz çıkarmak yeni bir anahtar dönemini zorunlu kılar.** Hiçbir şeyi değiştirmeyen roster
  reddedilir.
- Bir cihaz kendini çıkaramaz (kendi ürettiği yeni anahtarı bilirdi). Onu başka bir cihazdan
  çıkarmak gerekir.

Bu, ROADMAP'teki "cihaz listesi"dir: `GroupState::devices()` her cihaz için adı, parmak izini ve
"bu cihaz mı" bilgisini verir. Arayüz ve CLI yüzeyi, `panora-sync` süreciyle birlikte SYNC-04'te
gelecek.

### 4. Grup anahtarı

32 baytlık rastgele bir anahtar kullanılır. Roster anahtarın kendisini değil, anahtarlı bir BLAKE3
**denetim değerini** (`key_check`) ve anahtarın hangi epoch'ta geldiğini (`key_epoch`) taşır. Anahtar
yalnızca kimliği doğrulanmış bir kanaldan dağıtılır: yeni cihaza `Welcome` içinde, diğer üyelere
ise SYNC-04'ün sabitlenmiş TLS'i üzerinden (`key_for_peer` yalnızca güncel üyeye verir). Alınan
anahtar denetim değerine uymazsa reddedilir.

Grup anahtarıyla mühürlenen veri (`Sealed`) şunları taşır: anahtar epoch'u, denetim değerinin ilk
8 baytı ve `panora_core` zarfı. AAD = grup kimliği ‖ epoch ‖ anahtar kimliği ‖ bağlam
(`"sync-record"` vb.). **Açarken yalnızca güncel anahtar kabul edilir.** Çıkarılan cihaz eski
anahtarı hâlâ bildiği için onunla veri enjekte edememeli. Yolda kalan kayıtlar yeniden gönderilir
(senkron durum tabanlı olduğu için bu zararsızdır).

### 5. Eşzamanlı değişiklik (çatal) kuralı

İki üye aynı anda değişiklik yaparsa aynı ebeveynden iki **dal** doğar. Tek tek roster'lar değil,
dallar bütün olarak karşılaştırılır (`fork_winner`). Bunun nedeni şu: çıkarılmış bir cihaz,
hâlâ üye olduğu daha eski bir ebeveynin üzerine rakip bir roster imzalayabilir. Roster'ları tek
tek karşılaştırmak, iç incelemede bu yolla çıkarılmanın geri alınabildiğini gösterdi.

1. Diğer dalın imzalayanlarından birini çıkaran dal kazanır. Böylece bir çıkarma, hedefinin
   imzaladığı bir roster'la geri alınamaz.
2. İki dal da birbirinin imzalayanını çıkarıyorsa (iki cihaz birbirini çıkarıyor ya da çıkarılan
   cihaz karşılık veriyor) **cihaz elindeki dalı korur**: gördüğü bir çıkarmayı asla geri almaz.
   Burada sonuç her cihazda aynı olmayabilir. Hangi tarafı önce gördüyse orada kalır ve bu bir
   bölünmeye yol açabilir. Deterministik bir kural (ör. hash) kullanılsaydı, çıkarılan cihaz çatal
   noktasını seçerek kuralı kendi lehine çevirebilirdi. Bu yüzden güvenlik yakınsamaya tercih edildi.
3. Diğer dalın hâlâ listelediği bir cihazı çıkaran dal, kimseyi çıkarmayan dala karşı kazanır.
   Böylece çevrimdışı kalmış dürüst bir cihazın ilgisiz bir değişikliği (ör. bir cihaz eklemesi),
   çıkarılmış bir cihazı gruba geri sokamaz. İkinci iç inceleme bunun mümkün olduğunu gösterdi:
   çıkarılan cihaz, geri döndüğü bu dalın üzerine sahibini çıkaran bir roster imzalayıp sahibi
   kalıcı olarak dışarıda bırakabiliyordu.
4. Aksi hâlde imzalayanların etkileyemeyeceği bir değer karar verir: ebeveynin hash'i ve ilk
   roster'ı imzalayanın kimliğinin BLAKE3'ü, eşitlikte roster hash'i. Her cihaz aynı sonucu bulur.
   İki dal da birini çıkarıyorsa kaybedenin çıkarmaları geri verilir (aşağıya bakın) ve hemen
   yeniden yapılır.

Karşılaştırma dal büyüdükçe değişebilir. Örneğin çıkarılmayı görmemiş bir cihaz, çıkarılan cihazın
masum görünen bir çatalını seçebilir. Sahibin dalı (çıkarma dahil) ulaştığında o dal kazanır ve
cihaz ona geçer; bu durum testte gösterildi. Bu nedenle roster'lar zincir olarak uygulanır
(`apply_chain`): zincir cihazda olanla örtüşebilir, cihazın elindeki en eski roster'dan daha eski
bir noktadan ayrılan zincir ise açık bir hatayla reddedilir (sessizce yok sayılmaz). Yeni bir
cihaz `Welcome` içinde tutulan roster'ların hepsini (en fazla 8) alır. Böylece eski üyeler kadar
derindeki çatalları o da değerlendirebilir.

Çatalı kaybeden daldan yeniden yapılacaklar cihaza geri verilir
(`RosterUpdate::Replaced { reapply }`), **dalın son hâline göre süzülerek**; dalın ilerisinde
başkasının çıkardığı bir cihaz yeniden eklenmez. Geri verilenler:

- Cihazın **kendi** eklemeleri: bunları yalnızca o bilir, birkaç cihazın aynı eklemeyi yeniden
  yapması da yeniden çatal üretir.
- Kim yapmış olursa olsun **bütün** çıkarmalar: yalnızca eşitlik bozmada kaybetmiş bir çıkarma,
  çıkarılan cihazı, çıkarmayı yapan cihaz yeniden çevrimiçi olana kadar grupta bırakmamalı.
  Çatalı gören ilk üye çıkarmayı yeniden yapar.

`GroupState::reapply` bunları kazananın üzerine uygular; çağıran bunu hemen yapmalıdır. Kaybeden dalda eklenmiş bir
cihaz, yeniden eklenene kadar "üye değil ama çıkarılmadı" durumunda bekler. Anahtarlarını
yalnızca gelen dal onu açıkça çıkardığında siler (`RemovedThisDevice`).

### 6. Kalıcı durum

Kimlik (PKCS#8), cihaz adı ve grup (roster'lar + anahtarlar) tek bir JSON belgesinde tutulur. Bu
belge `panora_core` zarfıyla mühürlenir (AAD `panora-sync-state/1`). Geçici dosya önce silinip
`O_EXCL` ile ve `0600` izniyle yeniden yaratılır, sonra atomik olarak yerine taşınır. Gizli
alanların (grup anahtarı, PKCS#8) base64 metni kullanıldıktan sonra bellekten silinir. Yüklerken her roster ve zincir yeniden doğrulanır, her anahtarın bir roster'a
uyduğu denetlenir. Mühür anahtarı Secret Service'ten gelir (`panod`'un ana anahtarı gibi); bu
bağlantı SYNC-04'teki süreçte yapılacak.

### 7. Bağımlılık seçimi

- **`ring` 0.17:** Ed25519 ve X25519 için kullanılıyor. ADR 0004'teki quinn/rustls zaten `ring`
  getirecekti, bu yüzden ikinci bir kripto yığını gerekmiyor. Linux hedefinde yeni crate yalnızca
  `ring` ve `untrusted` (lisanslar Apache-2.0 AND ISC ve ISC, `deny.toml` içinde). Kilit
  dosyasındaki `windows-*` crate'leri Linux'ta derlenmiyor.
- **BLAKE3 ve XChaCha20-Poly1305** zaten workspace'te vardı. HKDF/HMAC/SHA-2 eklenmedi.
- **SPAKE2 (parola doğrulamalı anahtar anlaşması) alınmadı.** Kısa bir parolanın bir cihaza
  yazılmasını gerektirir ve `curve25519-dalek` yığınını getirirdi. Sayısal karşılaştırma
  kamerasız masaüstü akışına zaten uyuyor ve kullanıcıların Bluetooth eşleştirmesinden ve
  Signal güvenlik numaralarından tanıdığı bir yöntem. QR modunda ise 256 bitlik sır zaten var.

## Sonuçlar

- **Olumlu:** Eşleştirme, cihaz listesi, anahtar dönemleri ve mühürleme ağdan bağımsız olarak
  test edildi: 50 birim testi ve gerçek TCP soketleri üzerinden 4 uçtan uca test.
  Senaryolar: davetle katılma, kodla katılma, reddetme, yanlış mod, çıkarma ve anahtar yenileme.
  Saldırı testleri: araya girme, sızmış sır, bozulmuş commitment, düşük dereceli anahtar,
  sahte/yetkisiz/boşluklu roster, eski ebeveyn üzerinden geri dönme girişimi (40 farklı deneme),
  yarıda bırakılan oturumlarla kod öğütme, tek kullanımlık davet, süresi dolan pencere, epoch
  taşması ve eski anahtarla enjeksiyon. `panod`'un ağ yüzeyi hâlâ sıfır.
- **İç inceleme:** Protokol, commit'ten önce ayrı bir ajan tarafından saldırgan gözüyle
  incelendi. Bulgular ve kapanışları: çıkarılan cihazın eski ebeveyn üzerinden geri dönmesi
  (kritik; dal düzeyinde karşılaştırma), yarıda bırakılan oturumlarla kod öğütme (yüksek; başlayan
  her oturum sayılıyor, tek oturum), kaybeden daldan yanlış yeniden uygulama (orta), dar geçmiş
  yüzünden sessiz bölünme (orta; tam geçmiş ve açık hata), epoch taşması, gizli base64 metninin
  silinmemesi ve geçici dosya izni (düşük). Düzeltmelerden sonraki ikinci inceleme yedisinin de
  kapandığını doğruladı ve üç yeni bulgu çıkardı:
  - Çıkarılan cihazın, çevrimdışı dürüst bir cihazın ilgisiz değişikliği üzerinden geri dönüp
    sahibi dışarıda bırakması (yüksek). Kapatıldı: §5 madde 3 ve bütün çıkarmaların yeniden
    yapılması.
  - Pencere süresinin onay anında denetlenememesi (düşük). Kapatıldı: `run_inviter` artık bir
    saat alıyor ve saati onay anında okuyor.
  - Geride kalan bir eşin tam eşleşen zincirinin reddedilmesi (düşük). Kapatıldı: cihazın
    geçmişinden eski baştaki roster'lar atlanıyor.

  İncelemenin kavram kanıtı testleri depoya regresyon testi olarak eklendi. Bu incelemeler
  SYNC-06'daki bağımsız incelemenin yerini tutmaz.
- **Olumlu (yan düzeltme):** `panod`'un `device-id` dosyası artık 128 bit rastgele değer. Önceden
  süreç kimliği, saat ve veri yolunun hash'iydi; aynı kullanıcı adı ve yeni kurulumla iki makinede
  çakışabilirdi. Eşleştirme aynı device_id'li ikinci cihazı reddediyor (`Abort{refused}`). Klonlanmış
  bir sistem imajı kimliği dosyayla birlikte kopyalar; bu durumda dosyayı silmek yeter.
- **Olumsuz / bilinçli sınırlar:**
  - Kayıt başına imza yok. Bir üye grup anahtarıyla başka bir üyenin `device_id`'siyle kayıt
    mühürleyebilir. Tehdit modelinde üyeler zaten tam güvenilir.
  - Cihaz yeniden adlandırılamaz ve kendi kendine gruptan çıkamaz.
  - "Çıkarma savaşı": ele geçirilmiş bir üye diğerlerini çıkarabilir. Birbirini çıkaran iki dal
    arasında cihazlar önce gördüklerinde kalır (§5, madde 2). Çıkarılmayı görmüş cihazlar güvende
    kalır; görmemiş olanlar saldırganın tarafında kalabilir. Kullanıcı bu durumda yeni bir grup
    kurar. SYNC-04, bir çıkarmayı ulaşabildiği tüm üyelere hemen iletmeli.
  - Katılan cihaz davet süresini kendi saatiyle denetler; asıl sınırı davet eden uygular.
- **Açık iş:**
  - SYNC-04: `panora-sync` süreci, mDNS ile keşif, dinleyici ve zaman aşımı, sabitlenmiş TLS,
    roster/anahtar yayını ve durum anahtarının Secret Service'ten alınması.
  - Arayüz: QR gösterimi (GUI'de `qrcode` zaten var), kod onay penceresi, cihaz listesi.
  - SYNC-05: Android'in QR taraması.
  - SYNC-06: bağımsız güvenlik incelemesi.
