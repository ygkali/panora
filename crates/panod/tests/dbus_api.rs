// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! The public D-Bus API (INT-05) against a real session bus and a real
//! daemon: every method call over `io.github.ygkali.Panora1`, and the
//! `Changed` signal firing on a capture. Ignored by default for the same
//! reason as `tests/keyring.rs` (needs a real bus, not a mock); run
//! through `scripts/dbus-test.sh` or:
//!
//! ```sh
//! cargo test -p panod --test dbus_api -- --ignored
//! ```

#![cfg(unix)]

use futures::StreamExt as _;
use panod::daemon::Daemon;
use panora_core::backend::{ClipboardBackend, ClipboardEvent, MockBackend};
use panora_core::config::Config;
use panora_core::model::{ClipboardData, MimePayload, Selection};
use panora_core::storage::{BlobStore, Cipher, Database, MasterKey};
use panora_core::sync::NoopSync;
use std::os::unix::fs::MetadataExt as _;
use std::rc::Rc;
use std::sync::Arc;
use tokio::net::UnixListener;
use zbus::zvariant::OwnedValue;

/// Binds the *real* socket path (`XDG_RUNTIME_DIR`/`panora.sock`) and
/// starts serving it, so `dbus_api::run`'s internal IPC client (which
/// resolves that same path) reaches this daemon. Only ever one of these
/// per process: `XDG_RUNTIME_DIR` is process-global, which is exactly why
/// this test file exists on its own and is run with `--test-threads=1`.
async fn start(runtime_dir: &std::path::Path) -> (Rc<Daemon>, Arc<MockBackend>) {
    std::env::set_var("XDG_RUNTIME_DIR", runtime_dir);
    let backend = Arc::new(MockBackend::new());
    let key = MasterKey::generate();
    let db = Database::open(runtime_dir.join("history.db"), Cipher::new(&key)).unwrap();
    let blobs = BlobStore::open(runtime_dir.join("blobs"), Cipher::new(&key)).unwrap();
    let daemon = Rc::new(Daemon::new(
        backend.clone(),
        db,
        blobs,
        Config::default(),
        Arc::new(NoopSync),
        "dbus-test-device".into(),
    ));
    let socket = panora_core::config::socket_path();
    let listener = UnixListener::bind(&socket).unwrap();
    let uid = std::fs::metadata(&socket).unwrap().uid();
    let serving = daemon.clone();
    tokio::task::spawn_local(async move {
        let _ = panod::server::serve(listener, serving, uid).await;
    });
    tokio::task::spawn_local(async move {
        let _ = panod::dbus_api::run().await;
    });

    // Capture one entry through the mock backend, the same way the IPC
    // tests do, so List/Status have something to report.
    backend
        .offer(
            Selection::Clipboard,
            ClipboardData {
                selection: Selection::Clipboard,
                payloads: vec![MimePayload::new("text/plain", b"dbus api test entry")],
                offered_mimes: vec!["text/plain".into()],
                source_app: Some("test".into()),
            },
        )
        .await
        .unwrap();
    daemon
        .handle_event(ClipboardEvent::changed(
            Selection::Clipboard,
            vec!["text/plain".into()],
            Some("test".into()),
        ))
        .await;
    (daemon, backend)
}

/// Give a just-spawned local task a moment to actually run (own its bus
/// name, start listening) before the test proxy dials in.
async fn settle() {
    for _ in 0..50 {
        tokio::task::yield_now().await;
    }
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
}

#[tokio::test]
#[ignore = "needs a D-Bus session bus"]
async fn public_api_round_trips_over_the_real_session_bus() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let dir = tempfile::tempdir().unwrap();
            let (_daemon, _backend) = start(dir.path()).await;
            settle().await;

            let connection = zbus::Connection::session().await.unwrap();
            let proxy = zbus::Proxy::new(
                &connection,
                panod::dbus_api::BUS_NAME,
                panod::dbus_api::OBJECT_PATH,
                panod::dbus_api::BUS_NAME,
            )
            .await
            .unwrap();

            let status: std::collections::HashMap<String, OwnedValue> =
                proxy.call("Status", &()).await.unwrap();
            let entries: i64 = status.get("entries").unwrap().try_into().unwrap();
            assert_eq!(entries, 1);
            let backend: &str = status.get("backend").unwrap().try_into().unwrap();
            assert_eq!(backend, "mock");

            let filters: std::collections::HashMap<String, OwnedValue> =
                std::collections::HashMap::new();
            let list: Vec<std::collections::HashMap<String, OwnedValue>> =
                proxy.call("List", &(filters,)).await.unwrap();
            assert_eq!(list.len(), 1);
            let preview: &str = list[0].get("preview").unwrap().try_into().unwrap();
            assert_eq!(preview, "dbus api test entry");
            let id: i64 = list[0].get("id").unwrap().try_into().unwrap();

            let (): () = proxy.call("Pin", &(id, true)).await.unwrap();
            let filters: std::collections::HashMap<String, OwnedValue> =
                std::collections::HashMap::new();
            let list: Vec<std::collections::HashMap<String, OwnedValue>> =
                proxy.call("List", &(filters,)).await.unwrap();
            let pinned: bool = list[0].get("pinned").unwrap().try_into().unwrap();
            assert!(pinned);

            let pasted: bool = proxy
                .call("Recall", &(id, false, String::new()))
                .await
                .unwrap();
            assert!(!pasted, "paste was not requested");

            let (): () = proxy.call("Delete", &(id,)).await.unwrap();
            let filters: std::collections::HashMap<String, OwnedValue> =
                std::collections::HashMap::new();
            let list: Vec<std::collections::HashMap<String, OwnedValue>> =
                proxy.call("List", &(filters,)).await.unwrap();
            assert!(list.is_empty(), "Delete removed the entry");
        })
        .await;
}

#[tokio::test]
#[ignore = "needs a D-Bus session bus"]
async fn changed_signal_fires_on_a_capture() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let dir = tempfile::tempdir().unwrap();
            let (daemon, backend) = start(dir.path()).await;
            settle().await;

            let connection = zbus::Connection::session().await.unwrap();
            let proxy = zbus::Proxy::new(
                &connection,
                panod::dbus_api::BUS_NAME,
                panod::dbus_api::OBJECT_PATH,
                panod::dbus_api::BUS_NAME,
            )
            .await
            .unwrap();
            let mut signals = proxy.receive_signal("Changed").await.unwrap();

            // A second capture bumps the revision, which the background
            // Subscribe forwarder should turn into a Changed signal.
            backend
                .offer(
                    Selection::Clipboard,
                    ClipboardData {
                        selection: Selection::Clipboard,
                        payloads: vec![MimePayload::new("text/plain", b"second entry")],
                        offered_mimes: vec!["text/plain".into()],
                        source_app: Some("test".into()),
                    },
                )
                .await
                .unwrap();
            daemon
                .handle_event(ClipboardEvent::changed(
                    Selection::Clipboard,
                    vec!["text/plain".into()],
                    Some("test".into()),
                ))
                .await;

            let signal = tokio::time::timeout(std::time::Duration::from_secs(5), signals.next())
                .await
                .expect("a Changed signal within 5s")
                .expect("the signal stream stayed open");
            let (revision,): (u64,) = signal.body().deserialize().unwrap();
            assert!(revision > 0);
        })
        .await;
}
