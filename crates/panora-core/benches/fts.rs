// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! STO-06: FTS5 prefix search over a realistically sized history.
//! `docs/benchmark.md` records the measured numbers this produces.

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use panora_core::model::{ContentKind, Selection};
use panora_core::storage::{Cipher, Database, MasterKey, QueryFilter};

const ENTRY_COUNT: i64 = 10_000;

fn seeded_database() -> Database {
    let db = Database::open_in_memory(Cipher::new(&MasterKey::generate())).unwrap();
    for i in 0..ENTRY_COUNT {
        let preview = format!(
            "panora benchmark entry number {i} about topic{} with some extra words to search",
            i % 137
        );
        let hash = panora_core::storage::content_hash(preview.as_bytes());
        db.upsert_entry(
            &hash,
            &preview,
            ContentKind::Text,
            "text/plain",
            preview.len() as i64,
            Some("bench"),
            Selection::Clipboard,
            i,
            "bench-device",
            i,
            true,
        )
        .unwrap();
    }
    db
}

fn fts_prefix_search(c: &mut Criterion) {
    let db = seeded_database();
    let filter = QueryFilter {
        search: Some("topic5".into()),
        limit: 50,
        ..Default::default()
    };
    c.bench_function(
        &format!("fts prefix search, {ENTRY_COUNT} rows, 50 results"),
        |b| {
            b.iter(|| {
                let results = db.query(black_box(&filter)).unwrap();
                black_box(results);
            });
        },
    );
}

criterion_group!(benches, fts_prefix_search);
criterion_main!(benches);
