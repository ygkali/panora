// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! STO-06: the encrypted blob store in isolation (no database involved) —
//! `BlobStore::put`/`get`, the pair `panod::daemon::Daemon::load_payloads`
//! and image capture rely on for anything larger than a short text
//! preview. `docs/benchmark.md` records the measured numbers this
//! produces.

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use panora_core::storage::{BlobStore, Cipher, MasterKey};

/// Representative of a small captured image, larger than v3's inline
/// threshold (STO-08) so it also stands in for what would travel as a
/// passed fd over IPC.
const PAYLOAD_SIZE: usize = 256 * 1024;

fn open_store(dir: &tempfile::TempDir) -> BlobStore {
    BlobStore::open(
        dir.path().join("blobs"),
        Cipher::new(&MasterKey::generate()),
    )
    .unwrap()
}

fn blob_put(c: &mut Criterion) {
    let dir = tempfile::tempdir().unwrap();
    let store = open_store(&dir);
    let mut counter = 0u64;

    c.bench_function(&format!("blob put, {} KiB", PAYLOAD_SIZE / 1024), |b| {
        b.iter(|| {
            // Varies per iteration for the same reason store.rs's does:
            // identical bytes would dedup to the same file after the
            // first write, measuring an `exists()` check instead of a
            // real encrypt-and-write.
            counter += 1;
            let mut payload = vec![0x42u8; PAYLOAD_SIZE];
            payload[..8].copy_from_slice(&counter.to_le_bytes());
            let hash = store.put(black_box(&payload)).unwrap();
            black_box(hash);
        });
    });
}

fn blob_get(c: &mut Criterion) {
    let dir = tempfile::tempdir().unwrap();
    let store = open_store(&dir);
    let hash = store.put(&vec![0x42u8; PAYLOAD_SIZE]).unwrap();

    c.bench_function(&format!("blob get, {} KiB", PAYLOAD_SIZE / 1024), |b| {
        b.iter(|| {
            let data = store.get(black_box(&hash)).unwrap();
            black_box(data);
        });
    });
}

criterion_group!(benches, blob_put, blob_get);
criterion_main!(benches);
