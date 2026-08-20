# Panora benchmark raporu

**Ölçüm tarihi:** 2026-08-18. **Makine:** sandbox x86_64, Rust release profile, SQLite in-memory benchmark veritabanı.

## FTS5 arama

Criterion ile 10.000 pano kaydı oluşturuldu ve 50 sonuç limitli `merhaba` sorgusu ölçüldü. Sonuç: **15.675–15.737 ms (ortalama 15.705 ms)**. Proje hedefi olan 50 ms altında kalmaktadır.

Komut:

```sh
cargo bench -p panora-core --bench fts
```

Bu ölçüm yalnızca FTS5 arama yolunu kapsar; şifreli BLOB okuma, GUI render ve IPC gecikmesi dahil değildir. Benchmark kaynak dosyası `crates/panora-core/benches/fts.rs` içindedir.

## Bellek ve CPU

Daemon event loop ve GTK popup ayrı süreçlerdir. Boşta CPU için polling yerine backend olayları hedeflenmiştir; mevcut wl-clipboard-rs yüksek seviyeli API'si olay callback'i sunmadığı için Wayland watcher v1'de 120 ms düşük maliyetli değişiklik kontrolü kullanır. Bu, gelecekte doğrudan data-control event queue ile değiştirilecek optimizasyon noktasıdır. Bu nedenle **"event-driven, polling yok" hedefi Wayland v1 için henüz karşılanmış sayılmamalıdır**.

Boşta RAM hedefi <30 MB, kabul tavanı 50 MB'dir. Gerçek GNOME oturumu sandbox'ta bulunmadığından bu metrik release ortamında ayrıca ölçülmelidir:

```sh
/usr/bin/time -v -- target/release/panod
/usr/bin/time -v -- target/release/panora-gui
```

Daemon'un kullanıcı keyring'i ve GNOME/Wayland oturumu olmadan başlayamaması beklenen bir çevre koşuludur; bu bir performans ölçümü değildir.
