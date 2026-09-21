// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! STO-06: recording one entry end to end (`Database::upsert_entry` +
//! `BlobStore::put`), the same two calls `panod::daemon::Daemon::store`
//! makes on every real capture. `docs/benchmark.md` records the measured
//! numbers this produces.

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use panora_core::model::{ContentKind, Selection};
use panora_core::storage::{BlobStore, Cipher, Database, MasterKey};

/// A representative short clipboard text, matching `docs/benchmark.md`'s
/// "1 KiB" target.
fn sample_text() -> String {
    "panora ".repeat(1024 / "panora ".len() + 1)[..1024].to_string()
}

fn store_one_entry(c: &mut Criterion) {
    let dir = tempfile::tempdir().unwrap();
    let key = MasterKey::generate();
    let db = Database::open(dir.path().join("history.db"), Cipher::new(&key)).unwrap();
    let blobs = BlobStore::open(dir.path().join("blobs"), Cipher::new(&key)).unwrap();
    let text = sample_text();
    let mut counter = 0i64;

    c.bench_function("store one 1 KiB text entry (db + blob)", |b| {
        b.iter(|| {
            // A fresh piece of text per iteration: a repeat of the same
            // bytes would just dedup-update the same row after the first
            // one, measuring a cheap UPDATE instead of a real insert.
            counter += 1;
            let unique_text = format!("{text}{counter}");
            let hash = panora_core::storage::content_hash(unique_text.as_bytes());
            let blob_ref = blobs.put(unique_text.as_bytes()).unwrap();
            let id = db
                .upsert_entry(
                    black_box(&hash),
                    black_box(&unique_text),
                    ContentKind::Text,
                    "text/plain",
                    unique_text.len() as i64,
                    Some("bench"),
                    Selection::Clipboard,
                    counter,
                    "bench-device",
                    counter,
                )
                .unwrap();
            db.attach_blob(id, "text/plain", &blob_ref).unwrap();
            black_box(id);
        });
    });
}

criterion_group!(benches, store_one_entry);
criterion_main!(benches);
