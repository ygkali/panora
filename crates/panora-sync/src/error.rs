// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! Error type for the sync crate.

use crate::pairing::AbortReason;
use thiserror::Error;

/// Result alias for the crate.
pub type Result<T> = std::result::Result<T, Error>;

/// Everything that can go wrong while pairing or maintaining a group.
///
/// Authentication failures deliberately say *what* check failed but never
/// carry key material, transcript hashes or ciphertext.
#[derive(Debug, Error)]
pub enum Error {
    /// The peer sent a message that is malformed or out of order.
    #[error("pairing protocol error: {0}")]
    Protocol(&'static str),

    /// A cryptographic check failed: wrong invitation secret, broken
    /// commitment, bad signature, or a peer that is not who it claims.
    #[error("authentication failed: {0}")]
    Auth(&'static str),

    /// An invitation link could not be parsed, or has expired.
    #[error("invalid invitation: {0}")]
    Invitation(&'static str),

    /// A device roster or group key was rejected.
    #[error("roster rejected: {0}")]
    Roster(&'static str),

    /// A device name or id is not acceptable.
    #[error("invalid device: {0}")]
    Device(&'static str),

    /// The two devices were set to different pairing modes (one scanned an
    /// invitation, the other waits for a code, or the reverse).
    #[error("the devices are in different pairing modes")]
    ModeMismatch,

    /// The user on this device declined.
    #[error("pairing was cancelled on this device")]
    Cancelled,

    /// The other side ended the pairing.
    #[error("the other device aborted pairing ({0})")]
    Aborted(AbortReason),

    /// The QUIC/TLS layer failed: binding, dialling, a closed connection.
    #[error("network error: {0}")]
    Transport(String),

    /// `panod` could not be reached or refused a request.
    #[error("clipboard daemon: {0}")]
    Panod(String),

    /// A primitive in `ring` failed (key generation, agreement). Carries no
    /// detail on purpose.
    #[error("cryptographic operation failed")]
    Crypto,

    /// Reading or writing the state file or a pairing stream failed.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// A message or the state file is not valid JSON of the expected shape.
    #[error("encoding error: {0}")]
    Json(#[from] serde_json::Error),

    /// Sealing or opening with `panora_core`'s AEAD failed.
    #[error(transparent)]
    Core(#[from] panora_core::Error),
}
