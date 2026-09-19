// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! Network-free synchronization extension point.

use crate::error::Result;
use crate::model::Entry;
use async_trait::async_trait;

/// Events that a future encrypted sync transport may consume.
#[derive(Debug, Clone)]
pub enum SyncEvent {
    /// A history entry was created or refreshed.
    EntryUpserted(Entry),
    /// An entry was deleted locally.
    EntryDeleted {
        /// Local database identifier.
        id: i64,
        /// Stable BLAKE3 content hash.
        content_hash: String,
    },
    /// Pin state changed.
    PinChanged {
        /// Local database identifier.
        id: i64,
        /// New pin state.
        pinned: bool,
    },
}

/// Future synchronization provider contract.
#[async_trait]
pub trait SyncProvider: Send + Sync {
    /// Whether this provider is currently active.
    fn active(&self) -> bool;
    /// Receive a local event.
    async fn on_event(&self, event: SyncEvent);
}

/// Disabled v1 provider. It never opens a socket or performs network I/O.
#[derive(Debug, Default)]
pub struct NoopSync;

#[async_trait]
impl SyncProvider for NoopSync {
    fn active(&self) -> bool {
        false
    }

    async fn on_event(&self, _event: SyncEvent) {}
}

/// A small helper used by future transports to report a disabled result.
pub fn disabled_result<T>() -> Result<T> {
    Err(crate::error::Error::Config(
        "sync transport is disabled in Panora v1".into(),
    ))
}
