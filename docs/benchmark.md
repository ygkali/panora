# Panora benchmark notları

> **Durum (2026-09-19):** Önceki sürümde bu belge var olmayan bir `benches/fts.rs` dosyasına ve 2026-08-18 tarihli tek bir sandbox ölçümüne dayanıyordu. Criterion benchmark'ları (`STO-06`) ve gerçek makine ölçümleri (`STO-07`) `docs/ROADMAP.md` içinde planlıdır; sonuçlar alındığında bu dosya ölçüm tarihi, makine ve komutla birlikte yeniden yazılacaktır.

## Hedefler

| Metrik | Hedef | Kabul tavanı | Nasıl ölçülür |
|---|---|---|---|
| FTS5 önek araması, 10 000 kayıt, 50 sonuç | < 20 ms | 50 ms | criterion `panora-core` benchmark'ı |
| Kayıt saklama (metin, 1 KiB) | < 5 ms | 20 ms | criterion (`Database::upsert_entry` + `BlobStore::put`) |
| IPC gidiş-dönüş (`Status`) | < 1 ms | 5 ms | criterion, Unix soket, `MockBackend` |
| `panod` boşta RSS | < 30 MB | 50 MB | `/usr/bin/time -v` veya `smem`, gerçek oturum |
| Popup soğuk açılış (Super+V → pencere görünür) | < 400 ms | 800 ms | `GTK_DEBUG=interactive` değil; `journalctl` + zaman damgası, gerçek oturum |
| Popup sıcak açılış (UI-14 sonrası) | < 50 ms | 150 ms | aynı |

## Ölçüm yöntemi (planlanan)

```sh
cargo bench -p panora-core            # fts, store, blob
cargo bench -p panod                  # ipc roundtrip
/usr/bin/time -v -- target/release/panod   # gerçek oturumda, keyring açıkken
```

Daemon olay tabanlıdır (XFIXES / data-control); Wayland'de yoklama yoktur, bu nedenle boşta CPU kullanımı ölçülebilir bir hedef değil, `0` beklentisidir.
