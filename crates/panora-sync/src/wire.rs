// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! Running a pairing over a byte stream: length-prefixed JSON frames and
//! two drivers that walk [`Joiner`] and [`Inviter`] through the exchange.
//!
//! The stream is whatever SYNC-04 opens (a QUIC stream on the LAN); the
//! channel needs no security of its own, the protocol provides it. The
//! drivers have no timeout: a caller wraps them in one, generous enough for
//! a user to compare two codes.

use crate::error::{Error, Result};
use crate::group::{GroupState, Roster};
use crate::identity::PublicIdentity;
use crate::pairing::{AbortReason, Inviter, JoinRequest, Joiner, PairMessage, SasCode};
use std::future::Future;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Largest frame accepted. The biggest message, a welcome carrying eight
/// full rosters, stays well under it.
pub const MAX_FRAME: usize = 256 * 1024;

/// Write one message as a 4-byte big-endian length and JSON.
pub async fn write_message<W: AsyncWrite + Unpin>(writer: &mut W, msg: &PairMessage) -> Result<()> {
    let json = serde_json::to_vec(msg)?;
    if json.len() > MAX_FRAME {
        return Err(Error::Protocol("message too large"));
    }
    writer.write_all(&(json.len() as u32).to_be_bytes()).await?;
    writer.write_all(&json).await?;
    writer.flush().await?;
    Ok(())
}

/// Read one message.
pub async fn read_message<R: AsyncRead + Unpin>(reader: &mut R) -> Result<PairMessage> {
    let mut len = [0u8; 4];
    reader.read_exact(&mut len).await?;
    let len = u32::from_be_bytes(len) as usize;
    if len > MAX_FRAME {
        return Err(Error::Protocol("message too large"));
    }
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf).await?;
    Ok(serde_json::from_slice(&buf)?)
}

/// What to tell the other side when this side fails; `None` when there is
/// nobody to tell (the stream broke) or they already know (they aborted).
fn abort_reason(err: &Error) -> Option<AbortReason> {
    match err {
        Error::Io(_) | Error::Aborted(_) => None,
        Error::Cancelled => Some(AbortReason::Rejected),
        Error::ModeMismatch => Some(AbortReason::WrongMode),
        Error::Invitation(_) => Some(AbortReason::Closed),
        Error::Device(_) => Some(AbortReason::Refused),
        _ => Some(AbortReason::Failed),
    }
}

async fn finish<S, T>(stream: &mut S, result: Result<T>) -> Result<T>
where
    S: AsyncWrite + Unpin,
{
    if let Err(err) = &result {
        if let Some(reason) = abort_reason(err) {
            // Best effort: the error that ended the pairing is what matters.
            let _ = write_message(stream, &PairMessage::Abort { reason }).await;
        }
    }
    result
}

/// Join a group over `stream`. `commit` is the message [`Joiner::start`]
/// returned. `confirm` is asked once the inviter is known, with the code to
/// compare in code mode (`None` in invitation mode, where it may simply
/// say yes); returning `false` cancels.
pub async fn run_joiner<S, F, Fut>(
    stream: &mut S,
    mut joiner: Joiner<'_>,
    commit: PairMessage,
    confirm: F,
) -> Result<GroupState>
where
    S: AsyncRead + AsyncWrite + Unpin,
    F: FnOnce(Option<SasCode>, PublicIdentity) -> Fut,
    Fut: Future<Output = bool>,
{
    let result = async {
        write_message(stream, &commit).await?;
        let offer = read_message(stream).await?;
        let reveal = joiner.on_offer(offer)?;
        write_message(stream, &reveal).await?;
        let inviter = joiner.inviter().ok_or(Error::Protocol("no inviter"))?;
        if !confirm(joiner.code(), inviter).await {
            joiner.reject();
            return Err(Error::Cancelled);
        }
        let join = joiner.confirm()?;
        write_message(stream, &join).await?;
        let welcome = read_message(stream).await?;
        joiner.on_welcome(welcome)
    }
    .await;
    finish(stream, result).await
}

/// Answer one joining device over `stream` and, if admitted, add it to
/// `group`. `clock` gives the current Unix time; it is read when the
/// device is admitted, so a window that expired while the user was
/// deciding refuses it. `show_code` is called as soon as the code exists (code mode
/// only) so it appears on both screens together; `approve` is asked once
/// the device has introduced itself. Returns the request and the new
/// roster, which the caller sends to the other members.
pub async fn run_inviter<S, C, F, Fut>(
    stream: &mut S,
    mut inviter: Inviter<'_>,
    group: &mut GroupState,
    clock: impl Fn() -> i64,
    show_code: C,
    approve: F,
) -> Result<(JoinRequest, Roster)>
where
    S: AsyncRead + AsyncWrite + Unpin,
    C: FnOnce(SasCode),
    F: FnOnce(Option<SasCode>, JoinRequest) -> Fut,
    Fut: Future<Output = bool>,
{
    let result = async {
        let commit = read_message(stream).await?;
        let offer = inviter.on_commit(commit)?;
        write_message(stream, &offer).await?;
        let reveal = read_message(stream).await?;
        inviter.on_reveal(reveal)?;
        if let Some(code) = inviter.code() {
            show_code(code);
        }
        let join = read_message(stream).await?;
        let request = inviter.on_join(join)?;
        if !approve(inviter.code(), request.clone()).await {
            inviter.reject();
            return Err(Error::Cancelled);
        }
        let (welcome, roster) = inviter.approve(group, clock())?;
        write_message(stream, &welcome).await?;
        Ok((request, roster))
    }
    .await;
    finish(stream, result).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn frames_round_trip_and_oversize_is_refused() {
        let (mut a, mut b) = tokio::io::duplex(1 << 20);
        let msg = PairMessage::Abort {
            reason: AbortReason::Closed,
        };
        write_message(&mut a, &msg).await.unwrap();
        assert_eq!(read_message(&mut b).await.unwrap(), msg);

        a.write_all(&((MAX_FRAME as u32) + 1).to_be_bytes())
            .await
            .unwrap();
        assert!(matches!(
            read_message(&mut b).await,
            Err(Error::Protocol(_))
        ));
    }

    #[tokio::test]
    async fn garbage_is_an_encoding_error() {
        let (mut a, mut b) = tokio::io::duplex(1024);
        a.write_all(&5u32.to_be_bytes()).await.unwrap();
        a.write_all(b"nope!").await.unwrap();
        assert!(matches!(read_message(&mut b).await, Err(Error::Json(_))));
    }
}
