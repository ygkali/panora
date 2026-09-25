// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! Three devices over real loopback TCP sockets: A starts a group, B joins
//! with A's invitation link, C joins through B by comparing codes, then A
//! removes C and the key moves on without it.

use panora_core::model::{MimePayload, Selection};
use panora_core::sync::{payload_hash, SyncRecord};
use panora_sync::wire::{run_inviter, run_joiner};
use panora_sync::{
    AbortReason, Error, GroupState, Invitation, Inviter, Joiner, PairingWindow, RosterUpdate,
    SasCode, SyncState,
};
use std::sync::{Arc, Mutex};
use tokio::net::{TcpListener, TcpStream};

const NOW: i64 = 1_800_000_000;

fn record(text: &str, device_id: &str) -> SyncRecord {
    let payloads = vec![MimePayload::new("text/plain", text)];
    SyncRecord {
        content_hash: payload_hash(&payloads),
        selection: Selection::Clipboard,
        source_app: None,
        created_at: NOW,
        last_seen_at: NOW,
        pinned: false,
        deleted: false,
        device_id: device_id.into(),
        lamport: 1,
        payloads,
    }
}

async fn listener() -> (TcpListener, std::net::SocketAddr) {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    (l, addr)
}

#[tokio::test]
async fn invitation_then_code_then_removal() {
    let a = SyncState::new("desktop").unwrap();
    let b = SyncState::new("laptop").unwrap();
    let c = SyncState::new("old netbook").unwrap();
    let (a_id, b_id, c_id) = (
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        "cccccccccccccccccccccccccccccccc",
    );
    let mut ga = GroupState::create(&a.identity, a_id, &a.device_name, NOW).unwrap();

    // --- B joins A with the invitation link (what the QR code encodes).
    let (l, addr) = listener().await;
    let invitation = Invitation::new(a.identity.public(), vec![addr], NOW);
    let link = invitation.to_uri();
    let mut window = PairingWindow::for_invitation(invitation);

    let parsed = Invitation::parse(&link).unwrap();
    let target = parsed.addrs[0];
    let (joiner, commit) =
        Joiner::start(&b.identity, b_id, &b.device_name, Some(parsed), NOW).unwrap();
    let a_identity = a.identity.public();
    let joining = async {
        let mut s = TcpStream::connect(target).await.unwrap();
        run_joiner(&mut s, joiner, commit, |code, inviter| async move {
            assert!(code.is_none(), "invitation mode shows no code");
            inviter == a_identity
        })
        .await
    };
    let inviting = async {
        let (mut s, _) = l.accept().await.unwrap();
        let inviter = Inviter::new(&a.identity, &ga, &mut window, NOW).unwrap();
        run_inviter(
            &mut s,
            inviter,
            &mut ga,
            || NOW,
            |_| panic!("no code in invitation mode"),
            |_, _| async { true },
        )
        .await
    };
    let (gb, admitted) = tokio::join!(joining, inviting);
    let mut gb = gb.unwrap();
    let (request, _) = admitted.unwrap();
    assert_eq!(request.name, "laptop");
    assert_eq!(ga.current().hash(), gb.current().hash());
    // The link works once: a successful pairing closed the window.
    assert!(Inviter::new(&a.identity, &ga, &mut window, NOW).is_err());

    // --- C joins B by comparing codes; the users see the same six digits.
    let (l, addr) = listener().await;
    let mut window = PairingWindow::for_code(NOW);
    let shown_on_b = Arc::new(Mutex::new(None::<SasCode>));
    let shown_on_c = Arc::new(Mutex::new(None::<SasCode>));
    let (joiner, commit) = Joiner::start(&c.identity, c_id, &c.device_name, None, NOW).unwrap();
    let joining = {
        let shown_on_c = shown_on_c.clone();
        async move {
            let mut s = TcpStream::connect(addr).await.unwrap();
            run_joiner(&mut s, joiner, commit, |code, _| async move {
                *shown_on_c.lock().unwrap() = code;
                true
            })
            .await
        }
    };
    let inviting = {
        let shown_on_b = shown_on_b.clone();
        let gb = &mut gb;
        let window = &mut window;
        let b = &b;
        async move {
            let (mut s, _) = l.accept().await.unwrap();
            let inviter = Inviter::new(&b.identity, gb, window, NOW).unwrap();
            run_inviter(
                &mut s,
                inviter,
                gb,
                || NOW,
                move |code| *shown_on_b.lock().unwrap() = Some(code),
                |code, _| async move { code.is_some() },
            )
            .await
        }
    };
    let (gc, admitted) = tokio::join!(joining, inviting);
    let mut gc = gc.unwrap();
    let (_, roster_with_c) = admitted.unwrap();
    let (on_b, on_c) = (
        shown_on_b.lock().unwrap().unwrap(),
        shown_on_c.lock().unwrap().unwrap(),
    );
    assert_eq!(on_b, on_c);

    // A hears about C from B, and everyone lists the same three devices.
    assert_eq!(
        ga.apply_roster(roster_with_c).unwrap(),
        RosterUpdate::Advanced
    );
    for g in [&ga, &gb, &gc] {
        let mut names: Vec<_> = g.devices().into_iter().map(|d| d.member.name).collect();
        names.sort();
        assert_eq!(names, ["desktop", "laptop", "old netbook"]);
        assert_eq!(g.devices().iter().filter(|d| d.this_device).count(), 1);
    }

    // Everyone reads what anyone seals.
    let sealed = gc.seal_record(&record("from C", c_id)).unwrap();
    assert_eq!(ga.open_record(&sealed).unwrap().payloads[0].data, b"from C");
    assert_eq!(gb.open_record(&sealed).unwrap().payloads[0].data, b"from C");

    // --- A removes C. B follows and gets the new key from A; C is out.
    let (removal, new_key) = ga
        .remove_member(&a.identity, &c.identity.public(), NOW + 60)
        .unwrap();
    assert_eq!(
        gb.apply_roster(removal.clone()).unwrap(),
        RosterUpdate::Advanced
    );
    assert!(gb.seal(b"x", b"y").is_err(), "no key yet");
    let for_b = ga.key_for_peer(&b.identity.public()).unwrap();
    assert!(ga.key_for_peer(&c.identity.public()).is_err());
    gb.accept_key(for_b).unwrap();
    assert_eq!(
        gc.apply_roster(removal).unwrap(),
        RosterUpdate::RemovedThisDevice
    );
    assert!(gc.accept_key(new_key).is_err());

    let after = ga.seal_record(&record("after C left", a_id)).unwrap();
    assert!(gb.open_record(&after).is_ok());
    assert!(gc.open_record(&after).is_err());
    // What C seals with the key it still remembers is refused.
    assert!(ga.open_record(&sealed).is_err());
}

#[tokio::test]
async fn a_declined_code_leaves_the_group_unchanged() {
    let a = SyncState::new("desktop").unwrap();
    let b = SyncState::new("laptop").unwrap();
    let mut ga = GroupState::create(&a.identity, "a1", "desktop", NOW).unwrap();
    let (l, addr) = listener().await;
    let mut window = PairingWindow::for_code(NOW);
    let (joiner, commit) = Joiner::start(&b.identity, "b1", "laptop", None, NOW).unwrap();
    let joining = async {
        let mut s = TcpStream::connect(addr).await.unwrap();
        run_joiner(&mut s, joiner, commit, |_, _| async { true }).await
    };
    let inviting = async {
        let (mut s, _) = l.accept().await.unwrap();
        let inviter = Inviter::new(&a.identity, &ga, &mut window, NOW).unwrap();
        // The user on A says the codes differ.
        run_inviter(
            &mut s,
            inviter,
            &mut ga,
            || NOW,
            |_| {},
            |_, _| async { false },
        )
        .await
    };
    let (joined, invited) = tokio::join!(joining, inviting);
    assert!(matches!(joined, Err(Error::Aborted(AbortReason::Rejected))));
    assert!(matches!(invited, Err(Error::Cancelled)));
    assert_eq!(ga.current().epoch, 0);
    assert_eq!(ga.devices().len(), 1);
}

#[tokio::test]
async fn a_joiner_in_the_wrong_mode_is_told_so() {
    let a = SyncState::new("desktop").unwrap();
    let b = SyncState::new("laptop").unwrap();
    let mut ga = GroupState::create(&a.identity, "a1", "desktop", NOW).unwrap();
    let (l, addr) = listener().await;
    let mut window = PairingWindow::for_code(NOW);
    // B scanned an (old) invitation, but A is waiting for a code.
    let inv = Invitation::new(a.identity.public(), vec![addr], NOW);
    let (joiner, commit) = Joiner::start(&b.identity, "b1", "laptop", Some(inv), NOW).unwrap();
    let joining = async {
        let mut s = TcpStream::connect(addr).await.unwrap();
        run_joiner(&mut s, joiner, commit, |_, _| async { true }).await
    };
    let inviting = async {
        let (mut s, _) = l.accept().await.unwrap();
        let inviter = Inviter::new(&a.identity, &ga, &mut window, NOW).unwrap();
        run_inviter(
            &mut s,
            inviter,
            &mut ga,
            || NOW,
            |_| {},
            |_, _| async { true },
        )
        .await
    };
    let (joined, invited) = tokio::join!(joining, inviting);
    assert!(matches!(
        joined,
        Err(Error::Aborted(AbortReason::WrongMode))
    ));
    assert!(matches!(invited, Err(Error::ModeMismatch)));
}

#[tokio::test]
async fn approval_after_the_window_expired_is_refused() {
    use std::sync::atomic::{AtomicI64, Ordering};
    let a = SyncState::new("desktop").unwrap();
    let b = SyncState::new("laptop").unwrap();
    let mut ga = GroupState::create(&a.identity, "a1", "desktop", NOW).unwrap();
    let (l, addr) = listener().await;
    let mut window = PairingWindow::for_code(NOW);
    let clock = AtomicI64::new(NOW);
    let (joiner, commit) = Joiner::start(&b.identity, "b1", "laptop", None, NOW).unwrap();
    let joining = async {
        let mut s = TcpStream::connect(addr).await.unwrap();
        run_joiner(&mut s, joiner, commit, |_, _| async { true }).await
    };
    let inviting = async {
        let (mut s, _) = l.accept().await.unwrap();
        let inviter = Inviter::new(&a.identity, &ga, &mut window, NOW).unwrap();
        run_inviter(
            &mut s,
            inviter,
            &mut ga,
            || clock.load(Ordering::SeqCst),
            |_| {},
            |_, _| async {
                // The user took an hour to decide.
                clock.store(NOW + 3600, Ordering::SeqCst);
                true
            },
        )
        .await
    };
    let (joined, invited) = tokio::join!(joining, inviting);
    assert!(matches!(invited, Err(Error::Invitation(_))), "{invited:?}");
    assert!(matches!(joined, Err(Error::Aborted(AbortReason::Closed))));
    assert_eq!(ga.devices().len(), 1);
}
