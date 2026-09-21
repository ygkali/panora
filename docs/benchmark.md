# Panora benchmark notları

> **Durum (2026-09-21, STO-06):** `crates/panora-core/benches/{fts,store,blob}.rs` ve
> `crates/panod/benches/ipc.rs` artık gerçekten var ve çalışıyor (`cargo bench --workspace`).
> Aşağıdaki tablonun ilk üç satırı bu turda **gerçekten ölçüldü** — WSL2 Ubuntu 24.04
> (bu depoyu geliştirmek için kullanılan makine, §"Ortam" altında) üzerinde, tek bir koşu.
> Gerçek makine sonuçları (bare-metal Linux, sanallaştırma yükü olmadan) farklı çıkabilir;
> CI'daki `benchmark` işi (`.github/workflows/ci.yml`, haftalık cron) her hafta yeniden ölçüp
> `bench-results` artifact'ini yayımlıyor — zamanla sapma buradan izlenebilir. Son iki satır
> (boşta RSS, popup açılış süresi) gerçek bir masaüstü oturumu gerektiriyor (`STO-07`); bu
> proje WSL/CI'da anlamlı ölçülemediği için roadmap'te bilinçli olarak atlandı, hâlâ
> "ölçüm bekleniyor".

## Sonuçlar

| Metrik | Hedef | Kabul tavanı | Ölçülen (WSL2, 2026-09-21) | Durum |
|---|---|---|---|---|
| FTS5 önek araması, 10 000 kayıt, 50 sonuç | < 20 ms | 50 ms | **~1,13 ms** | ✅ hedefin çok altında |
| Kayıt saklama (metin, 1 KiB, db+blob) | < 5 ms | 20 ms | **~5,5 ms** | ⚠️ hedefi hafif aşıyor, tavanın altında |
| IPC gidiş-dönüş (`Status`, gerçek soket) | < 1 ms | 5 ms | **~72 µs** | ✅ hedefin çok altında |
| Blob yaz (256 KiB) | — | — | ~6,6 ms | bilgi amaçlı, hedef yok |
| Blob oku (256 KiB) | — | — | ~204 µs | bilgi amaçlı, hedef yok |
| `panod` boşta RSS | < 30 MB | 50 MB | — | **ölçüm bekleniyor** (STO-07, gerçek oturum gerekir) |
| Popup soğuk açılış (Super+V → pencere görünür) | < 400 ms | 800 ms | — | **ölçüm bekleniyor** (STO-07) |
| Popup sıcak açılış (UI-14 sonrası) | < 50 ms | 150 ms | — | **ölçüm bekleniyor** (STO-07, UI-14 henüz yok) |

Kayıt saklama satırı hedefi (5 ms) hafifçe aşıyor; muhtemel neden SQLite'ın WAL
`fsync`'i + blob dosyası yazımının bu sanal makinedeki disk gecikmesi — kabul tavanının
(20 ms) yarısından az olduğu için P0 değil, ama bare-metal bir ölçüm bunu doğrulamalı.

## Ölçüm yöntemi

```sh
cargo bench -p panora-core            # fts, store, blob
cargo bench -p panod --bench ipc      # ipc roundtrip (gerçek Unix soket + MockBackend)
cargo bench --workspace --no-run      # yalnızca derleme kontrolü (CI'nin her push'ta yaptığı)

# STO-07 tamamlandığında, gerçek bir oturumda:
/usr/bin/time -v -- target/release/panod   # gerçek oturumda, keyring açıkken
```

Sonuçlar `target/criterion/` altına HTML raporu olarak da yazılır (gnuplot yoksa
plotters backend'iyle SVG). CI'daki `benchmark` işi bunu haftalık olarak `bench-results`
adıyla artifact'e yüklüyor; `bench-build` işi ise her push'ta yalnızca derlenebilirliği
kontrol ediyor (gerçek bir ölçüm koşusu paylaşılan bir runner'da anlamlı değil).

Daemon olay tabanlıdır (XFIXES / data-control); Wayland'de yoklama yoktur, bu nedenle boşta
CPU kullanımı ölçülebilir bir hedef değil, `0` beklentisidir.

## Ortam (2026-09-21 ölçümü)

- WSL2 Ubuntu 24.04, `rustc` stable, `cargo bench` (release + debug-assertions kapalı).
- `CARGO_TARGET_DIR` WSL'in kendi ext4 dosya sisteminde (`~/panora-target`), `/mnt/c`
  üzerinde değil — çapraz dosya sistemi G/Ç gecikmesi ölçümü etkilemiyor.
- Tek koşu, karşılaştırma/trend yok; `benchmark` CI işi zamanla veri biriktirecek.
