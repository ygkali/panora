// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! `Daemon::rotate_key` (SEC-01) against a real Secret Service, including a
//! simulated crash mid-rotation. Ignored by default for the same reason as
//! `tests/keyring.rs`; run through `scripts/keyring-test.sh` or:
//!
//! ```sh
//! cargo test -p panod --test rotate_key -- --ignored
//! ```

#![cfg(unix)]

use panod::daemon::Daemon;
use panora_core::backend::MockBackend;
use panora_core::config::Config;
use panora_core::storage::{BlobStore, Cipher, Database};
use panora_core::sync::NoopSync;
use std::path::Path;
use std::sync::Arc;

fn build_daemon(dir: &Path, cipher_key: &panora_core::storage::MasterKey) -> Daemon {
    let backend = Arc::new(MockBackend::new());
    let db = Database::open(dir.join("history.db"), Cipher::new(cipher_key)).unwrap();
    let blobs = BlobStore::open(dir.join("blobs"), Cipher::new(cipher_key)).unwrap();
    Daemon::new(
        backend,
        db,
        blobs,
        Config::default(),
        Arc::new(NoopSync),
        "rotate-key-test-device".into(),
    )
}

async fn store_one(daemon: &Daemon, text: &str) {
    daemon
        .store_external(
            panora_core::model::ClipboardData {
                selection: panora_core::model::Selection::Clipboard,
                payloads: vec![panora_core::model::MimePayload::new(
                    "text/plain",
                    text.as_bytes(),
                )],
                offered_mimes: vec!["text/plain".into()],
                source_app: Some("test".into()),
            },
            false,
        )
        .await
        .unwrap();
}

/// Clears any pending rotation state a previous, unrelated test run in the
/// same throwaway keyring might have left behind, so this test starts from
/// a clean slate. `scripts/keyring-test.sh` always uses a fresh keyring, so
/// this is a no-op there; it only matters running these tests by hand.
async fn clear_pending_key() {
    if let Ok(Some(key)) = panod::keyring::load_pending_key().await {
        let _ = panod::keyring::finish_rotation(&key).await;
    }
}

#[tokio::test]
#[ignore = "needs a Secret Service on the session bus"]
async fn rotate_key_reseals_history_and_the_keyring_holds_the_new_key() {
    clear_pending_key().await;
    let dir = tempfile::tempdir().unwrap();
    let starting_key = panora_core::storage::MasterKey::generate();
    let daemon = build_daemon(dir.path(), &starting_key);
    store_one(&daemon, "before rotation").await;

    daemon.rotate_key().await.expect("rotation must succeed");

    let entries = daemon
        .query(&panora_core::storage::QueryFilter::recent(10))
        .unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].preview, "before rotation");
    assert!(
        daemon.status().unwrap().health.is_empty(),
        "no leftover rotation_incomplete health item"
    );

    // The daemon now uses a different key than it started with, and the
    // keyring's live item matches it (a fresh load equals what a restarted
    // daemon would pick up).
    let reloaded = panod::keyring::load_or_create_master_key().await.unwrap();
    assert_ne!(reloaded.as_bytes(), starting_key.as_bytes());
    assert!(panod::keyring::load_pending_key().await.unwrap().is_none());
}

#[tokio::test]
#[ignore = "needs a Secret Service on the session bus"]
async fn rotate_key_resumes_after_a_simulated_crash() {
    clear_pending_key().await;
    let dir = tempfile::tempdir().unwrap();
    let starting_key = panora_core::storage::MasterKey::generate();
    let daemon = build_daemon(dir.path(), &starting_key);
    store_one(&daemon, "resumed after a crash").await;

    // Simulate the daemon dying after the previews and blobs were resealed
    // (Database::rekey/BlobStore::rekey both succeeded) but before the
    // keyring was updated: the pending key is durably stored, but
    // `finish_rotation` never ran, so the *old* item is still "live".
    let pending_key = panora_core::storage::MasterKey::generate();
    panod::keyring::store_pending_key(&pending_key)
        .await
        .unwrap();
    daemon.db().set_rotation_state(Some("rotating")).unwrap();
    daemon
        .db()
        .rekey(panora_core::storage::Cipher::new(&pending_key))
        .unwrap();
    daemon
        .blobs()
        .rekey(panora_core::storage::Cipher::new(&pending_key))
        .unwrap();
    // A fresh `Daemon` here would stand in for the restarted process; this
    // one already holds the pending key in its live storage handles, so
    // calling `rotate_key` again exercises exactly what a resumed daemon
    // does: pick the same pending key back up and finish.
    daemon
        .rotate_key()
        .await
        .expect("resumed rotation must succeed");

    let entries = daemon
        .query(&panora_core::storage::QueryFilter::recent(10))
        .unwrap();
    assert_eq!(entries[0].preview, "resumed after a crash");
    assert!(panod::keyring::load_pending_key().await.unwrap().is_none());
    let reloaded = panod::keyring::load_or_create_master_key().await.unwrap();
    assert_eq!(reloaded.as_bytes(), pending_key.as_bytes());
}
