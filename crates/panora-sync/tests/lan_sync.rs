// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! End to end over real sockets: a real `panod` per device (mock clipboard
//! backend, temporary directory, its own IPC socket) and a sync node per
//! device talking QUIC over loopback. Pairing by invitation and by code,
//! entries, deletions and pins travelling both ways, and a removed device
//! that stops receiving.

#![cfg(unix)]

use panod::daemon::Daemon;
use panod::server;
use panora_core::backend::MockBackend;
use panora_core::config::Config;
use panora_core::ipc::QueryRequest;
use panora_core::model::{ClipboardData, Entry, MimePayload, Selection};
use panora_core::storage::{BlobStore, Cipher, Database, MasterKey, QueryFilter};
use panora_core::sync::NoopSync;
use panora_sync::node::{Node, NodeConfig, PairEvent};
use panora_sync::SyncState;
use std::os::unix::fs::MetadataExt as _;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::UnixListener;

struct Device {
    daemon: Rc<Daemon>,
    node: Node,
    state_path: PathBuf,
    _dir: tempfile::TempDir,
}

/// The state key every test device uses (its keyring stand-in).
const STATE_KEY: [u8; 32] = [9; 32];

async fn device(name: &str, device_id: &str) -> Device {
    let dir = tempfile::tempdir().unwrap();
    let key = MasterKey::generate();
    let db = Database::open(dir.path().join("history.db"), Cipher::new(&key)).unwrap();
    let blobs = BlobStore::open(dir.path().join("blobs"), Cipher::new(&key)).unwrap();
    let daemon = Rc::new(Daemon::new(
        Arc::new(MockBackend::new()),
        db,
        blobs,
        Config::default(),
        Arc::new(NoopSync),
        device_id.into(),
    ));
    let socket: PathBuf = dir.path().join("panora.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let uid = std::fs::metadata(&socket).unwrap().uid();
    let serving = daemon.clone();
    tokio::task::spawn_local(async move {
        let _ = server::serve(listener, serving, uid).await;
    });
    let node = Node::start(
        NodeConfig {
            listen: "127.0.0.1:0".parse().unwrap(),
            peers: Vec::new(),
            discovery: false,
            panod_socket: socket,
            state_path: dir.path().join("sync-state.bin"),
            device_id: device_id.into(),
            scope: Default::default(),
            manage_config: false,
        },
        SyncState::new(name).unwrap(),
        MasterKey::from_bytes(STATE_KEY),
    )
    .await
    .unwrap();
    Device {
        daemon,
        node,
        state_path: dir.path().join("sync-state.bin"),
        _dir: dir,
    }
}

async fn copy(device: &Device, text: &str) -> Entry {
    device
        .daemon
        .store(ClipboardData {
            selection: Selection::Clipboard,
            payloads: vec![MimePayload::new("text/plain", text.as_bytes())],
            offered_mimes: vec!["text/plain".into()],
            source_app: Some("test".into()),
        })
        .await
        .unwrap()
}

fn find(device: &Device, text: &str) -> Option<Entry> {
    device
        .daemon
        .query(&QueryFilter::from(QueryRequest::default()))
        .unwrap()
        .into_iter()
        .find(|e| e.preview == text)
}

async fn eventually(what: &str, mut check: impl FnMut() -> bool) {
    for _ in 0..200 {
        if check() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("timed out waiting for: {what}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_devices_pair_and_sync_both_ways() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let a = device("desktop", "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").await;
            let b = device("laptop", "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb").await;

            // Something A had before pairing travels too.
            copy(&a, "copied before pairing").await;

            let (mut invitation, mut events, _) = a.node.invite().await.unwrap();
            // Loopback, not the machine's LAN addresses: the test endpoints
            // listen on 127.0.0.1.
            invitation.addrs = vec![a.node.local_addr()];
            b.node.join(invitation).await.unwrap();
            assert!(matches!(events.recv().await, Some(PairEvent::Joined(_))));

            eventually("a session between A and B", || {
                a.node.connected() == vec![b.node.identity()]
                    && b.node.connected() == vec![a.node.identity()]
            })
            .await;
            assert_eq!(a.node.status().devices.len(), 2);

            eventually("the old entry on B", || {
                find(&b, "copied before pairing").is_some()
            })
            .await;
            copy(&a, "hello from A").await;
            copy(&b, "hello from B").await;
            eventually("A's entry on B", || find(&b, "hello from A").is_some()).await;
            eventually("B's entry on A", || find(&a, "hello from B").is_some()).await;

            // Deleting on B deletes on A; pinning on A pins on B.
            let on_b = find(&b, "hello from A").unwrap();
            b.daemon.delete(on_b.id).await.unwrap();
            eventually("the deletion on A", || find(&a, "hello from A").is_none()).await;
            let on_a = find(&a, "hello from B").unwrap();
            a.daemon.set_pinned(on_a.id, true).await.unwrap();
            eventually("the pin on B", || {
                find(&b, "hello from B").is_some_and(|e| e.pinned)
            })
            .await;

            // A removes B: the session closes, B learns it is out, and what
            // A copies afterwards stays on A.
            let fingerprint = b.node.status().fingerprint;
            a.node.remove(&fingerprint[..9]).unwrap();
            eventually("B out of the group", || !b.node.status().member).await;
            eventually("no session", || a.node.connected().is_empty()).await;
            copy(&a, "after the removal").await;
            tokio::time::sleep(Duration::from_secs(2)).await;
            assert!(find(&b, "after the removal").is_none());
            assert!(a.node.status().has_key);
            assert!(!b.node.status().has_key);

            a.node.shutdown();
            b.node.shutdown();
        })
        .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_stranger_learns_nothing_about_the_group() {
    use panora_sync::transport::{open_authenticated, Transport, ALPN_SYNC};
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let a = device("desktop", "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").await;
            a.node.invite().await.unwrap();
            // Any device can authenticate as itself; that does not make it
            // a member, and a member says nothing until it proves it is one.
            let stranger = Transport::bind("127.0.0.1:0".parse().unwrap()).unwrap();
            let identity = panora_sync::DeviceIdentity::generate().unwrap();
            let conn = stranger
                .connect(a.node.local_addr(), ALPN_SYNC)
                .await
                .unwrap();
            let (_send, mut recv, peer) = open_authenticated(&conn, &identity).await.unwrap();
            assert_eq!(peer, a.node.identity());
            // A device joins while the stranger listens: the new roster is
            // broadcast to sessions, and must not reach this one.
            let b = device("laptop", "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb").await;
            let (mut invitation, _events, _) = a.node.invite().await.unwrap();
            invitation.addrs = vec![a.node.local_addr()];
            let mut first = [0u8; 1];
            let listening = tokio::time::timeout(
                Duration::from_secs(4),
                tokio::io::AsyncReadExt::read(&mut recv, &mut first),
            );
            let (heard, joined) = tokio::join!(listening, b.node.join(invitation));
            joined.unwrap();
            // Silence, or the session closing: never a byte of data.
            assert!(
                !matches!(heard, Ok(Ok(n)) if n > 0),
                "the member sent something to a stranger"
            );
            a.node.shutdown();
            b.node.shutdown();
        })
        .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_third_device_joins_by_code_and_catches_up() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let a = device("desktop", "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").await;
            let b = device("laptop", "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb").await;
            let c = device("netbook", "cccccccccccccccccccccccccccccccc").await;

            let (mut invitation, _events, _) = a.node.invite().await.unwrap();
            invitation.addrs = vec![a.node.local_addr()];
            b.node.join(invitation).await.unwrap();
            copy(&b, "from B, before C").await;

            // C joins through B by comparing codes; B's user says yes after
            // seeing the same code C's user sees.
            let (mut events, _) = b.node.open_code_window().await.unwrap();
            let shown_on_b = tokio::spawn(async move {
                let mut shown = None;
                while let Some(event) = events.recv().await {
                    match event {
                        PairEvent::Code(code) => shown = Some(code),
                        PairEvent::Request { code, reply, .. } => {
                            assert_eq!(code, shown);
                            let _ = reply.send(true);
                        }
                        PairEvent::Joined(_) => return shown,
                        PairEvent::Failed { message, .. } => {
                            panic!("pairing failed on B: {message}")
                        }
                    }
                }
                shown
            });
            let seen_on_c = Arc::new(std::sync::Mutex::new(None));
            let seen = seen_on_c.clone();
            c.node
                .join_by_code(Some(b.node.local_addr()), |code, _| async move {
                    *seen.lock().unwrap() = Some(code);
                    true
                })
                .await
                .unwrap();
            let shown = shown_on_b.await.unwrap();
            assert!(shown.is_some());
            assert_eq!(shown, *seen_on_c.lock().unwrap());

            // A learns about C through B's roster, and C gets both A's and
            // B's history, whichever device it happens to be connected to.
            a.node.dial(c.node.local_addr());
            eventually("A lists C", || a.node.status().devices.len() == 3).await;
            copy(&a, "from A, after C").await;
            eventually("B's older entry on C", || {
                find(&c, "from B, before C").is_some()
            })
            .await;
            eventually("A's entry on C", || find(&c, "from A, after C").is_some()).await;

            for d in [&a, &b, &c] {
                d.node.shutdown();
            }
        })
        .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_entry_is_passed_on_by_the_device_in_the_middle() {
    // C and B never meet: C is connected to A only, B joins A later with
    // an entry of its own. What A applies from B must still reach C, even
    // though C has already read far past B's (low) Lamport value on A.
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let a = device("desktop", "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").await;
            let b = device("laptop", "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb").await;
            let c = device("netbook", "cccccccccccccccccccccccccccccccc").await;
            let (mut invitation, _e, _) = a.node.invite().await.unwrap();
            invitation.addrs = vec![a.node.local_addr()];
            c.node.join(invitation).await.unwrap();
            for n in 0..10 {
                copy(&a, &format!("from A {n}")).await;
            }
            eventually("A's entries on C", || find(&c, "from A 9").is_some()).await;

            copy(&b, "from B, written before it joined").await;
            let (mut invitation, _e, _) = a.node.invite().await.unwrap();
            invitation.addrs = vec![a.node.local_addr()];
            b.node.join(invitation).await.unwrap();
            eventually("B's entry on A", || {
                find(&a, "from B, written before it joined").is_some()
            })
            .await;
            eventually("B's entry on C, through A", || {
                find(&c, "from B, written before it joined").is_some()
            })
            .await;
            assert!(!c.node.connected().contains(&b.node.identity()));
            for d in [&a, &b, &c] {
                d.node.shutdown();
            }
        })
        .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn strangers_cannot_use_up_a_code_window() {
    use panora_sync::transport::{Transport, ALPN_PAIR};
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let a = device("desktop", "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").await;
            let c = device("netbook", "cccccccccccccccccccccccccccccccc").await;
            let (mut events, _) = a.node.open_code_window().await.unwrap();
            // As many connections as the window has attempts (one address
            // may hold no more than four at once), saying nothing or
            // nonsense.
            let stranger = Transport::bind("127.0.0.1:0".parse().unwrap()).unwrap();
            let mut held = Vec::new();
            for n in 0..3 {
                let conn = stranger
                    .connect(a.node.local_addr(), ALPN_PAIR)
                    .await
                    .unwrap();
                let (mut send, _recv) = conn.open_bi().await.unwrap();
                if n % 2 == 0 {
                    tokio::io::AsyncWriteExt::write_all(&mut send, b"\0\0\0\x05hello")
                        .await
                        .unwrap();
                }
                held.push((conn, send));
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
            let answering = tokio::spawn(async move {
                while let Some(event) = events.recv().await {
                    match event {
                        PairEvent::Request { reply, .. } => {
                            let _ = reply.send(true);
                        }
                        PairEvent::Joined(_) => return true,
                        _ => {}
                    }
                }
                false
            });
            c.node
                .join_by_code(Some(a.node.local_addr()), |_, _| async { true })
                .await
                .unwrap();
            assert!(answering.await.unwrap());
            drop(held);
            a.node.shutdown();
            c.node.shutdown();
        })
        .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn after_leaving_a_device_starts_over_with_a_new_identity() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let a = device("desktop", "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").await;
            let before = a.node.identity();
            a.node.invite().await.unwrap();
            a.node.leave().unwrap();
            let after = a.node.identity();
            assert_ne!(before, after);
            // A new group is made with the new identity, and the state file
            // (what the next start reads) agrees.
            a.node.invite().await.unwrap();
            let state = SyncState::load(&a.state_path, &MasterKey::from_bytes(STATE_KEY))
                .unwrap()
                .unwrap();
            assert_eq!(state.identity.public(), after);
            assert_eq!(state.group.unwrap().me(), after);
            a.node.shutdown();
        })
        .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn nothing_is_lost_while_the_receiver_is_in_private_mode() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let a = device("desktop", "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").await;
            let b = device("laptop", "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb").await;
            let (mut invitation, _e, _) = a.node.invite().await.unwrap();
            invitation.addrs = vec![a.node.local_addr()];
            b.node.join(invitation).await.unwrap();
            eventually("a session", || !a.node.connected().is_empty()).await;

            b.daemon.set_private_mode(true);
            copy(&a, "sent while B was private").await;
            tokio::time::sleep(Duration::from_secs(2)).await;
            assert!(find(&b, "sent while B was private").is_none());
            b.daemon.set_private_mode(false);
            eventually("the entry once B records again", || {
                find(&b, "sent while B was private").is_some()
            })
            .await;
            a.node.shutdown();
            b.node.shutdown();
        })
        .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn closing_the_control_connection_withdraws_an_invitation() {
    // What the GUI's Back button does: the link it showed must stop
    // working at once, not ten minutes later.
    use panora_sync::control::{self, Client, Event, Failure, Request};
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let a = device("desktop", "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").await;
            let b = device("laptop", "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb").await;
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("control.sock");
            let (serving, at) = (a.node.clone(), path.clone());
            tokio::spawn(async move {
                let _ = control::serve(serving, &at).await;
            });
            eventually("the control socket", || path.exists()).await;

            let mut client = Client::connect(&path).await.unwrap();
            client.send(&Request::Invite).await.unwrap();
            let link = match client.next().await.unwrap() {
                Some(Event::Link { link, .. }) => link,
                other => panic!("expected a link, got {other:?}"),
            };
            drop(client);
            tokio::time::sleep(Duration::from_millis(500)).await;

            let mut invitation = panora_sync::Invitation::parse(&link).unwrap();
            invitation.addrs = vec![a.node.local_addr()];
            let joined = b.node.join(invitation).await;
            assert_eq!(
                joined.as_ref().map_err(|e| e.failure()),
                Err(Failure::NotOpen),
                "{joined:?}"
            );
            assert_eq!(a.node.status().devices.len(), 1);
            a.node.shutdown();
            b.node.shutdown();
        })
        .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_no_on_the_joining_device_reaches_the_inviter_as_a_rejection() {
    use panora_sync::control::Failure;
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let a = device("desktop", "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").await;
            let c = device("netbook", "cccccccccccccccccccccccccccccccc").await;
            let (mut events, _) = a.node.open_code_window().await.unwrap();
            let refused = c
                .node
                .join_by_code(Some(a.node.local_addr()), |_, _| async { false })
                .await;
            assert_eq!(refused.map_err(|e| e.failure()), Err(Failure::Cancelled));
            let failure = tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    match events.recv().await {
                        Some(PairEvent::Failed { failure, .. }) => return failure,
                        Some(_) => {}
                        None => panic!("the window closed"),
                    }
                }
            })
            .await
            .unwrap();
            assert_eq!(failure, Failure::Rejected);
            a.node.shutdown();
            c.node.shutdown();
        })
        .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn strangers_cannot_use_up_an_invitation() {
    // More pairing requests than an invitation window has attempts, from a
    // device that saw the mDNS announcement but not the link.
    use panora_sync::pairing::{PairMessage, PairingMode, PROTOCOL};
    use panora_sync::transport::{Transport, ALPN_PAIR};
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let a = device("desktop", "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").await;
            let b = device("laptop", "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb").await;
            let (mut invitation, _events, _) = a.node.invite().await.unwrap();
            invitation.addrs = vec![a.node.local_addr()];
            let stranger = Transport::bind("127.0.0.1:0".parse().unwrap()).unwrap();
            for _ in 0..(panora_sync::invite::MAX_INVITATION_ATTEMPTS + 2) {
                let conn = stranger
                    .connect(a.node.local_addr(), ALPN_PAIR)
                    .await
                    .unwrap();
                let (send, recv) = conn.open_bi().await.unwrap();
                let mut stream = tokio::io::join(recv, send);
                let commit = PairMessage::Commit {
                    protocol: PROTOCOL.into(),
                    mode: PairingMode::Invitation,
                    commitment: [7; 32],
                    proof: None,
                };
                panora_sync::wire::write_message(&mut stream, &commit)
                    .await
                    .unwrap();
                let answer = panora_sync::wire::read_message(&mut stream).await;
                assert!(
                    matches!(answer, Ok(PairMessage::Abort { .. }) | Err(_)),
                    "{answer:?}"
                );
            }
            // The link still works for the device that has it.
            b.node.join(invitation).await.unwrap();
            assert_eq!(a.node.status().devices.len(), 2);
            a.node.shutdown();
            b.node.shutdown();
        })
        .await;
}
