// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! `Daemon::wipe` (SEC-03) against a real Secret Service: it retires the
//! key that encrypted the wiped history for a fresh one, which the
//! database/blob-store-level tests can't exercise on their own. Ignored by
//! default for the same reason as `tests/keyring.rs`; run through
//! `scripts/keyring-test.sh` or:
//!
//! ```sh
//! cargo test -p panod --test wipe -- --ignored
//! ```

#![cfg(unix)]

use panod::daemon::Daemon;
use panora_core::backend::MockBackend;
use panora_core::config::Config;
use panora_core::model::{ClipboardData, MimePayload, Selection};
use panora_core::storage::{BlobStore, Cipher, Database, MasterKey};
use panora_core::sync::NoopSync;
use std::path::Path;
use std::sync::Arc;

fn build_daemon(dir: &Path, key: &MasterKey) -> Daemon {
    let backend = Arc::new(MockBackend::new());
    let db = Database::open(dir.join("history.db"), Cipher::new(key)).unwrap();
    let blobs = BlobStore::open(dir.join("blobs"), Cipher::new(key)).unwrap();
    Daemon::new(
        backend,
        db,
        blobs,
        Config::default(),
        Arc::new(NoopSync),
        "wipe-test-device".into(),
    )
}

async fn store_one(daemon: &Daemon, text: &str) -> i64 {
    daemon
        .store_external(
            ClipboardData {
                selection: Selection::Clipboard,
                payloads: vec![MimePayload::new("text/plain", text.as_bytes())],
                offered_mimes: vec!["text/plain".into()],
                source_app: Some("test".into()),
            },
            false,
        )
        .await
        .unwrap()
        .id
}

async fn clear_keyring_state() {
    if let Ok(Some(key)) = panod::keyring::load_pending_key().await {
        let _ = panod::keyring::finish_rotation(&key).await;
    }
    let _ = panod::keyring::remove_lock_backup().await;
}

#[tokio::test]
#[ignore = "needs a Secret Service on the session bus"]
async fn wipe_clears_history_and_the_keyring_ends_up_with_a_different_key() {
    clear_keyring_state().await;
    let dir = tempfile::tempdir().unwrap();
    let starting_key = panod::keyring::load_or_create_master_key().await.unwrap();
    let daemon = build_daemon(dir.path(), &starting_key);

    let pinned_id = store_one(&daemon, "pinned, still not spared by wipe").await;
    daemon.set_pinned(pinned_id, true).await.unwrap();
    store_one(&daemon, "ordinary entry").await;
    daemon
        .set_lock_password(Some("about to be wiped away"), None)
        .await
        .unwrap();
    daemon.engage_lock().unwrap();

    daemon.wipe().await.expect("wipe must succeed");

    assert_eq!(
        daemon.db().count().unwrap(),
        0,
        "pinned entries are not spared"
    );
    assert!(!daemon.is_app_locked(), "wipe also disengages the lock");
    assert!(
        !daemon.lock_password_set().unwrap(),
        "and clears the password"
    );

    let reloaded = panod::keyring::load_or_create_master_key().await.unwrap();
    assert_ne!(
        reloaded.as_bytes(),
        starting_key.as_bytes(),
        "the keyring holds a fresh key, not the one that encrypted the wiped data"
    );
    assert!(panod::keyring::load_lock_backup().await.unwrap().is_none());
    assert!(panod::keyring::load_pending_key().await.unwrap().is_none());

    // The daemon's own storage already adopted the fresh key too (not just
    // the keyring): a capture right after wipe must round-trip normally.
    let id = store_one(&daemon, "captured after the wipe").await;
    let entries = daemon
        .query(&panora_core::storage::QueryFilter::recent(10))
        .unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].id, id);
    assert_eq!(entries[0].preview, "captured after the wipe");
}
