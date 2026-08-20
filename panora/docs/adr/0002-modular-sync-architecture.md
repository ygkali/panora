# ADR 0002: Modüler Senkron Mimarisi (Şimdi Stub, Sonra Modül)

- **Durum:** Kabul edildi
- **Tarih:** 2026-08-18

## Bağlam

Kullanıcı gereksinimi: cihazlar arası şifreli pano senkronizasyonu (ileride telefon dahil) hedefleniyor, ancak v1.0 kapsamında uygulanmayacak. Mimari, senkronun **sonradan ayrı bir modül/paket olarak** eklenebilmesini sağlamalı; mevcut kurulumları ve veritabanını kırmamalı.

## Karar

Senkron, dört genişletme noktasıyla mimariye gömülür ancak v1.0'da uygulanmaz:

1. **`SyncProvider` trait'i + `NoopSync` stub'ı** (`panora-core::sync`): Daemon, depolama olaylarını (yeni kayıt, silme, pin değişimi) her zaman bir `SyncProvider`'a bildirir. v1.0'da bu `NoopSync`'tir (hiçbir şey yapmaz, sıfır maliyet).
2. **Cargo feature flag `sync`:** Kapalıyken hiçbir ağ bağımlılığı derlenmez; v1.0 binary'si tamamen çevrimdışıdır ve ağ izni gerektirmez.
3. **Şema hazırlığı:** `entries` tablosunda `device_id BLOB NOT NULL`, `lamport INTEGER NOT NULL`, `deleted INTEGER DEFAULT 0` (tombstone) sütunları ilk günden vardır. Senkron eklendiğinde **veritabanı migrasyonu gerekmez**.
4. **Paket ayrımı:** v1.0 `panora` çekirdek .deb'i olarak yayınlanır; senkron ileride `panora-sync` adlı ayrı .deb olarak gelir (çekirdeğe bağımlı, `apt install panora-sync` ile eklenebilir). D-Bus API'si `org.panora.Pano1` olarak versiyonlanır.

## Gerekçe

- **Kapsam disiplini:** Senkron, projenin en yüksek karmaşıklık riskidir (NAT delme, eşleştirme, çakışma çözümü). Çekirdek kararlı olmadan başlanması projeyi boğabilir.
- **Kırılmazlık:** Şema alanları ve trait sözleşmesi baştan tanımlı olduğundan, senkron modülü geriye dönük uyumlu eklenir; kullanıcı verisi korunur.
- **Test edilebilirlik:** `NoopSync` ve mock `SyncProvider` ile daemon'un senkron bildirim yolları v1.0'da test edilir; modül geldiğinde entegrasyon noktaları zaten doğrulanmış olur.

## Gelecek Modül İçin Öngörülen Tasarım (bağlayıcı değil)

- Taşıma: **iroh** (QUIC + NAT delme, Ed25519 cihaz kimliği, relay fallback)
- İçerik şifreleme: XChaCha20-Poly1305, eşleştirmede türetilen grup anahtarı (relay güvenilmez varsayımı)
- Çakışma çözümü: Lamport saati + son-yazan-kazanır + tombstone (CRDT yok — pano geçmişi append-only)
- Eşleştirme: QR kod + 6 haneli doğrulama parmak izi
- Telefon: Android companion (Quick Settings tile ile manuel gönderim; Android 10+ arka plan pano kısıtı nedeniyle)

## Sonuçlar

- **Olumlu:** v1.0 küçük ve kararlı kalır; senkron eklendiğinde migrasyon/kırılma yok; ağ yüzeyi v1.0'da sıfır (güvenlik denetimi kolay).
- **Olumsuz:** Şemada üç kullanılmayan sütun taşınır (ihmal edilebilir maliyet); trait sözleşmesi değişirse modül uyumu gerekir.
