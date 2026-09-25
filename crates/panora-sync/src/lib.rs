// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! Panora device sync: pairing, the device roster and the group key
//! (ROADMAP SYNC-02, ADR 0005).
//!
//! Nothing here opens a socket. The pairing protocol is a pair of sans-IO
//! state machines ([`Joiner`], [`Inviter`]) plus drivers that run them over
//! any byte stream ([`wire`]); the LAN transport and the `panora-sync`
//! process that uses it come with SYNC-04 (ADR 0004). `panod` does not
//! depend on this crate and keeps its `AF_UNIX`-only sandbox.
//!
//! - [`DeviceIdentity`]: each device's Ed25519 key, pinned by the others.
//! - [`Invitation`] / [`PairingWindow`]: the QR code or link a member shows,
//!   and how long and how often it accepts a new device.
//! - [`Joiner`] / [`Inviter`]: `panora-pair/1`, with an invitation or by
//!   comparing a six-digit code.
//! - [`GroupState`]: the signed roster (the device list), the shared key,
//!   key rotation when a device is removed, concurrent-change resolution,
//!   and sealing records under the group key.
//! - [`SyncState`]: all of the above in one encrypted file.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod bytes;
/// The control socket of the running service.
pub mod control;
/// Finding the other devices with mDNS.
pub mod discovery;
/// Error type.
pub mod error;
/// Framing of the sync channel.
pub mod frame;
/// The roster, the group key and sealing.
pub mod group;
/// Device identity keys.
pub mod identity;
/// Invitations and pairing windows.
pub mod invite;
/// The Secret Service item that keeps the state key.
pub mod keyring;
/// The sync node: sessions, pairing, discovery.
pub mod node;
/// The pairing protocol state machines.
pub mod pairing;
/// `panod` client.
pub mod panod;
/// One sync session with one peer.
mod session;
/// Persistent, encrypted sync state.
pub mod state;
/// QUIC on the local network and peer authentication.
pub mod transport;
/// Framing and stream drivers for pairing.
pub mod wire;

pub use error::{Error, Result};
pub use group::{Change, DeviceInfo, GroupKey, GroupState, Member, Roster, RosterUpdate, Sealed};
pub use identity::{DeviceIdentity, PublicIdentity};
pub use invite::{Invitation, PairingWindow};
pub use pairing::{AbortReason, Inviter, JoinRequest, Joiner, PairMessage, PairingMode, SasCode};
pub use state::SyncState;
