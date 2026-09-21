// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! `Daemon::set_lock_password` (SEC-02) against a real Secret Service: it
//! writes a password-gated backup copy of the master key there, on top of
//! the database-side verifier `crates/panod/tests/ipc_server.rs` already
//! covers without one. Ignored by default for the same reason as
//! `tests/keyring.rs`; run through `scripts/keyring-test.sh` or:
//!
//! ```sh
//! cargo test -p panod --test lock -- --ignored
//! ```

#![cfg(unix)]

use panod::daemon::Daemon;
use panora_core::backend::MockBackend;
use panora_core::config::Config;
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
        "lock-test-device".into(),
    )
}

/// Clears keyring state a previous, unrelated test run in the same
/// throwaway keyring might have left behind. `scripts/keyring-test.sh`
/// always uses a fresh keyring, so this only matters running by hand.
async fn clear_keyring_state() {
    if let Ok(Some(key)) = panod::keyring::load_pending_key().await {
        let _ = panod::keyring::finish_rotation(&key).await;
    }
    let _ = panod::keyring::remove_lock_backup().await;
}

#[tokio::test]
#[ignore = "needs a Secret Service on the session bus"]
async fn set_lock_password_writes_a_keyring_backup_the_password_can_recover() {
    clear_keyring_state().await;
    let dir = tempfile::tempdir().unwrap();
    let key = panod::keyring::load_or_create_master_key().await.unwrap();
    let daemon = build_daemon(dir.path(), &key);

    daemon
        .set_lock_password(Some("first password"), None)
        .await
        .expect("setting the first password needs no current one");
    assert!(daemon.lock_password_set().unwrap());

    let wrapped = panod::keyring::load_lock_backup()
        .await
        .unwrap()
        .expect("a backup was written");
    let secret = daemon.db().lock_secret().unwrap().unwrap();
    let recovered = secret.unwrap_key("first password", &wrapped).unwrap();
    assert_eq!(recovered.as_bytes(), key.as_bytes());
    assert!(secret.unwrap_key("wrong password", &wrapped).is_err());
}

#[tokio::test]
#[ignore = "needs a Secret Service on the session bus"]
async fn changing_the_password_requires_the_current_one_and_rewraps_the_backup() {
    clear_keyring_state().await;
    let dir = tempfile::tempdir().unwrap();
    let key = panod::keyring::load_or_create_master_key().await.unwrap();
    let daemon = build_daemon(dir.path(), &key);
    daemon
        .set_lock_password(Some("old password"), None)
        .await
        .unwrap();

    assert!(
        daemon
            .set_lock_password(Some("new password"), Some("wrong current"))
            .await
            .is_err(),
        "the wrong current password must not be accepted"
    );

    daemon
        .set_lock_password(Some("new password"), Some("old password"))
        .await
        .expect("the right current password changes it");

    let wrapped = panod::keyring::load_lock_backup().await.unwrap().unwrap();
    let secret = daemon.db().lock_secret().unwrap().unwrap();
    let recovered = secret.unwrap_key("new password", &wrapped).unwrap();
    assert_eq!(recovered.as_bytes(), key.as_bytes());
}

#[tokio::test]
#[ignore = "needs a Secret Service on the session bus"]
async fn removing_the_password_clears_the_backup_and_disengages_the_lock() {
    clear_keyring_state().await;
    let dir = tempfile::tempdir().unwrap();
    let key = panod::keyring::load_or_create_master_key().await.unwrap();
    let daemon = build_daemon(dir.path(), &key);
    daemon
        .set_lock_password(Some("a password"), None)
        .await
        .unwrap();
    daemon.engage_lock().unwrap();
    assert!(daemon.is_app_locked());

    daemon
        .set_lock_password(None, Some("a password"))
        .await
        .unwrap();

    assert!(!daemon.lock_password_set().unwrap());
    assert!(
        !daemon.is_app_locked(),
        "removing the password also unlocks"
    );
    assert!(panod::keyring::load_lock_backup().await.unwrap().is_none());
}
