// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! One sync session with one authenticated peer, over the connection's
//! first bi-stream.
//!
//! Both sides open with `Hello` (their device id and every roster they
//! keep), then exchange rosters and keys as they change and push their own
//! change feed: each side reads `panod`'s feed from where the peer last
//! confirmed, seals the records under the group key and waits for
//! `Applied` before moving the peer's cursor. `Retry` leaves the cursor
//! where it was (the receiver lacked the key, or its daemon was locked or
//! down), so nothing is skipped.

use crate::error::{Error, Result};
use crate::frame;
use crate::group::{validate_device_id, GroupKey, Roster, Sealed};
use crate::identity::PublicIdentity;
use crate::node::Shared;
use panora_core::sync::{SyncCursor, SyncRecord};
use quinn::{Connection, RecvStream, SendStream};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, watch};
use tracing::{debug, info};

/// Largest frame a member may send: one page of the feed, sealed. `panod`
/// keeps a page under 32 MiB of payloads, or one entry of at most 48 MiB;
/// the rest is sealing and framing overhead. Two frames can be in flight,
/// so this also bounds what one member can make this device hold.
pub const MAX_SYNC_FRAME: usize = 64 * 1024 * 1024;
/// Largest first frame (the hello, eight rosters at most), read before the
/// other side has shown it is a member.
const MAX_HELLO_FRAME: usize = 512 * 1024;
/// Records asked of `panod` per push.
const PAGE: usize = 32;
/// How often an idle session looks at the feed anyway.
const TICK: Duration = Duration::from_secs(10);
/// How long a session waits for the other side's hello.
const PROBATION: Duration = Duration::from_secs(15);

/// Messages of the sync channel.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum Message {
    /// First message each way.
    Hello {
        device_id: String,
        rosters: Vec<Roster>,
    },
    /// The sender's roster chain changed.
    Rosters { rosters: Vec<Roster> },
    /// The sender lacks the current group key.
    KeyRequest,
    /// The current group key, for a member.
    KeyShare { key: GroupKey },
    /// Sealed records; ciphertexts travel as the frame's blobs, in order.
    Records { batch: u64, sealed: Vec<SealedHead> },
    /// The batch was applied; move my cursor.
    Applied { batch: u64 },
    /// The batch could not be applied now; send it again later.
    Retry { batch: u64 },
    /// The sender is closing the session.
    Bye { reason: String },
}

/// [`Sealed`] without its ciphertext, which rides as a blob.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct SealedHead {
    key_epoch: u64,
    #[serde(with = "crate::bytes::b64")]
    key_id: [u8; 8],
}

/// What the node asks of a running session.
#[derive(Debug)]
pub(crate) enum Command {
    Rosters(Vec<Roster>),
    KeyShare(GroupKey),
    Nudge,
    Close,
}

struct InFlight {
    batch: u64,
    next: SyncCursor,
    more: bool,
}

async fn send(stream: &mut SendStream, message: &Message, blobs: &[Vec<u8>]) -> Result<()> {
    frame::write(stream, message, blobs).await
}

/// Run a session until either side ends it. The caller has authenticated
/// `peer` and registered the session.
pub(crate) async fn run(
    shared: Arc<Shared>,
    conn: Connection,
    mut out: SendStream,
    recv: RecvStream,
    peer: PublicIdentity,
    mut commands: mpsc::UnboundedReceiver<Command>,
) -> Result<()> {
    // One frame in the queue: a member can make this side hold at most
    // two of them (the queued one and the one being read).
    let (inbox_tx, mut inbox) = mpsc::channel(1);
    let (trusted_tx, trusted) = watch::channel(false);
    let reader = tokio::spawn(read_loop(recv, inbox_tx, trusted));
    let result = session_loop(
        &shared,
        &conn,
        &mut out,
        peer,
        &mut inbox,
        &mut commands,
        &trusted_tx,
    )
    .await;
    if let Err(e) = &result {
        let _ = send(
            &mut out,
            &Message::Bye {
                reason: e.to_string(),
            },
            &[],
        )
        .await;
    }
    let _ = out.finish();
    reader.abort();
    result
}

async fn read_loop(
    mut recv: RecvStream,
    inbox: mpsc::Sender<(Message, Vec<Vec<u8>>)>,
    mut trusted: watch::Receiver<bool>,
) {
    // Any device can authenticate as itself. Until its hello has shown it
    // is a member, it gets one hello's worth of memory and nothing more is
    // read from it.
    let mut limit = MAX_HELLO_FRAME;
    loop {
        if limit == MAX_SYNC_FRAME && trusted.wait_for(|t| *t).await.is_err() {
            return;
        }
        match frame::read::<_, Message>(&mut recv, limit).await {
            Ok(frame) => {
                limit = MAX_SYNC_FRAME;
                if inbox.send(frame).await.is_err() {
                    return;
                }
            }
            Err(e) => {
                debug!(error = %e, "sync stream ended");
                return;
            }
        }
    }
}

async fn session_loop(
    shared: &Arc<Shared>,
    conn: &Connection,
    out: &mut SendStream,
    peer: PublicIdentity,
    inbox: &mut mpsc::Receiver<(Message, Vec<Vec<u8>>)>,
    commands: &mut mpsc::UnboundedReceiver<Command>,
    trusted: &watch::Sender<bool>,
) -> Result<()> {
    // The roster lists every member's name and keys: a device this one
    // does not know as a member must first show a chain that makes it one.
    let hello = || -> Result<Message> {
        Ok(Message::Hello {
            device_id: shared.config.device_id.clone(),
            rosters: shared.roster_chain().ok_or(Error::Roster("no group"))?,
        })
    };
    let mut sent_hello = false;
    if shared.is_peer(&peer) {
        send(out, &hello()?, &[]).await?;
        sent_hello = true;
    }
    let probation = tokio::time::sleep(PROBATION);
    tokio::pin!(probation);

    let mut changes = shared.changes();
    let mut tick = tokio::time::interval(TICK);
    let mut peer_device: Option<String> = None;
    let mut in_flight: Option<InFlight> = None;
    let mut want_push = true;
    let mut next_batch = 1u64;

    loop {
        if want_push && in_flight.is_none() && peer_device.is_some() {
            want_push = false;
            match push(shared, out, peer, peer_device.as_deref(), next_batch).await {
                Ok(Push::Sent(flight)) => {
                    next_batch += 1;
                    in_flight = Some(flight);
                }
                Ok(Push::CaughtUp { more }) => want_push = more,
                Ok(Push::NotNow) => {}
                Err(e @ Error::Transport(_)) | Err(e @ Error::Io(_)) => return Err(e),
                Err(e) => {
                    debug!(error = %e, "reading the change feed failed (daemon locked or down?); will retry")
                }
            }
        }
        tokio::select! {
            frame = inbox.recv() => {
                let Some((message, blobs)) = frame else { return Ok(()) };
                if peer_device.is_none() && !matches!(message, Message::Hello { .. }) {
                    return Err(Error::Protocol("expected a hello first"));
                }
                match message {
                    Message::Hello { device_id, rosters } => {
                        validate_device_id(&device_id)?;
                        shared.receive_rosters(peer, rosters);
                        if !shared.is_peer(&peer) {
                            return Err(Error::Roster("that device is not in this group"));
                        }
                        if shared.device_id_of(&peer).as_deref() != Some(device_id.as_str()) {
                            return Err(Error::Roster("the device id does not match the device list"));
                        }
                        peer_device = Some(device_id);
                        let _ = trusted.send(true);
                        if !sent_hello {
                            send(out, &hello()?, &[]).await?;
                            sent_hello = true;
                        }
                        if !shared.has_current_key() {
                            send(out, &Message::KeyRequest, &[]).await?;
                        }
                        want_push = true;
                    }
                    Message::Rosters { rosters } => {
                        shared.receive_rosters(peer, rosters);
                        if !shared.is_peer(&peer) {
                            return Err(Error::Roster("that device is no longer in this group"));
                        }
                        if !shared.has_current_key() {
                            send(out, &Message::KeyRequest, &[]).await?;
                        }
                    }
                    // Everything below needs a member on the other end; a
                    // device removed a moment ago gets and gives nothing.
                    _ if !shared.is_peer(&peer) => {
                        return Err(Error::Roster("that device is no longer in this group"));
                    }
                    Message::KeyRequest => {
                        if let Some(key) = shared.key_for(&peer) {
                            send(out, &Message::KeyShare { key }, &[]).await?;
                        }
                    }
                    Message::KeyShare { key } => shared.receive_key(key),
                    Message::Records { batch, sealed } => {
                        let reply = receive_records(shared, sealed, blobs).await;
                        let message = match reply {
                            Ok(()) => Message::Applied { batch },
                            Err(e) => {
                                debug!(error = %e, "could not apply a batch now");
                                if !shared.has_current_key() {
                                    send(out, &Message::KeyRequest, &[]).await?;
                                }
                                Message::Retry { batch }
                            }
                        };
                        send(out, &message, &[]).await?;
                    }
                    Message::Applied { batch } => {
                        if let Some(flight) = in_flight.take_if(|f| f.batch == batch) {
                            shared.set_cursor(peer, flight.next);
                            want_push = flight.more || want_push;
                        }
                    }
                    Message::Retry { batch } => {
                        if in_flight.as_ref().is_some_and(|f| f.batch == batch) {
                            in_flight = None;
                        }
                    }
                    Message::Bye { reason } => {
                        info!(peer = %peer.fingerprint(), %reason, "peer closed the session");
                        return Ok(());
                    }
                }
            }
            command = commands.recv() => match command {
                None | Some(Command::Close) => return Ok(()),
                // Only to a device that has shown it is a member: a session
                // still waiting for the other side's hello gets no roster
                // and no key, whatever changes meanwhile. A member removed
                // just now does get the roster that removes it (it knew
                // everything else in it), so it learns it is out.
                Some(Command::Rosters(rosters)) => {
                    if peer_device.is_some() {
                        send(out, &Message::Rosters { rosters }, &[]).await?;
                    }
                }
                Some(Command::KeyShare(key)) => {
                    if peer_device.is_some() && shared.is_peer(&peer) {
                        send(out, &Message::KeyShare { key }, &[]).await?;
                    }
                }
                Some(Command::Nudge) => want_push = true,
            },
            changed = changes.changed() => {
                if changed.is_err() {
                    return Ok(());
                }
                want_push = true;
            }
            _ = tick.tick() => want_push = true,
            _ = &mut probation, if peer_device.is_none() => {
                return Err(Error::Auth("the other device never showed it is a member"));
            }
            reason = conn.closed() => {
                debug!(peer = %peer.fingerprint(), %reason, "connection closed");
                return Ok(());
            }
        }
    }
}

enum Push {
    Sent(InFlight),
    CaughtUp { more: bool },
    NotNow,
}

async fn push(
    shared: &Arc<Shared>,
    out: &mut SendStream,
    peer: PublicIdentity,
    peer_device: Option<&str>,
    batch: u64,
) -> Result<Push> {
    if !shared.is_peer(&peer) || !shared.has_current_key() {
        return Ok(Push::NotNow);
    }
    let cursor = shared.cursor(&peer);
    let (records, next, more) = shared
        .panod
        .changes(cursor, PAGE, shared.config.scope)
        .await?;
    // A state the peer itself made, it already has (or something newer).
    let records: Vec<SyncRecord> = records
        .into_iter()
        .filter(|r| Some(r.device_id.as_str()) != peer_device)
        .collect();
    if records.is_empty() {
        if next != cursor {
            shared.set_cursor(peer, next);
        }
        return Ok(Push::CaughtUp { more });
    }
    let sealed = shared.seal_records(&records)?;
    let count = sealed.len();
    let (heads, blobs): (Vec<SealedHead>, Vec<Vec<u8>>) = sealed
        .into_iter()
        .map(|s| {
            (
                SealedHead {
                    key_epoch: s.key_epoch,
                    key_id: s.key_id,
                },
                s.ciphertext,
            )
        })
        .unzip();
    send(
        out,
        &Message::Records {
            batch,
            sealed: heads,
        },
        &blobs,
    )
    .await?;
    debug!(peer = %peer.fingerprint(), count, "sent records");
    Ok(Push::Sent(InFlight { batch, next, more }))
}

async fn receive_records(
    shared: &Arc<Shared>,
    heads: Vec<SealedHead>,
    blobs: Vec<Vec<u8>>,
) -> Result<()> {
    if heads.len() != blobs.len() {
        return Err(Error::Protocol("records and ciphertexts do not match up"));
    }
    let sealed: Vec<Sealed> = heads
        .into_iter()
        .zip(blobs)
        .map(|(h, ciphertext)| Sealed {
            key_epoch: h.key_epoch,
            key_id: h.key_id,
            ciphertext,
        })
        .collect();
    let records = shared.open_records(&sealed)?;
    // In private mode or behind the second-layer lock the daemon would
    // drop or refuse the records; ask for them again later instead.
    if shared.panod.paused().await? {
        return Err(Error::Panod("recording is paused".into()));
    }
    let (applied, ignored, rejected) = shared.panod.apply(records).await?;
    debug!(applied, ignored, rejected, "applied a batch");
    Ok(())
}
