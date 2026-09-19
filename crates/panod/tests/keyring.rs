// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! Secret Service integration. Ignored by default because it needs a live
//! `org.freedesktop.secrets` on the session bus; CI (and `scripts/
//! keyring-test.sh` locally) runs it under `dbus-run-session` with an
//! unlocked `gnome-keyring-daemon`:
//!
//! ```sh
//! cargo test -p panod --test keyring -- --ignored
//! ```

#![cfg(unix)]

#[tokio::test]
#[ignore = "needs a Secret Service on the session bus"]
async fn master_key_is_created_once_and_reloaded_unchanged() {
    let first = panod::keyring::load_or_create_master_key()
        .await
        .expect("a Secret Service must be reachable on the session bus");
    let second = panod::keyring::load_or_create_master_key()
        .await
        .expect("second load");
    assert_eq!(
        first.as_bytes(),
        second.as_bytes(),
        "the stored item must be found again instead of a new key being generated"
    );
    assert_ne!(first.as_bytes(), &[0u8; 32]);
}
