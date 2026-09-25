// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! Network-free synchronization extension point.

/// The control protocol of the `panora-sync` service.
pub mod control;

use crate::error::Result;
use crate::model::{ContentKind, Entry, MimePayload, Selection};
use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Lamport values from a peer must stay below this, far beyond any real
/// history (one change per microsecond for 285 years).
pub const LAMPORT_CEILING: i64 = 1 << 53;

/// The local clock follows a Lamport value only below this, half the
/// accepted range, whether the value arrives from a peer or sits in a row
/// a peer's state was applied to. Otherwise one peer could send a value
/// just under [`LAMPORT_CEILING`], this device's next change would land on
/// it, every other device would refuse that and all later changes, and
/// sync would stop for good without a word. With the gap, a device's own
/// values stay below the ceiling for 2^52 more changes. A value in the
/// upper half is still applied (the entry takes that state); it just does
/// not move the clock.
pub const LAMPORT_OBSERVE_LIMIT: i64 = 1 << 52;

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

/// The state of one history entry as it travels between devices (SYNC-03).
///
/// State-based rather than an operation log: a record always carries the
/// entry's *latest* state, and `(lamport, device_id)` says how new that
/// state is. An entry is identified across devices by `content_hash` and
/// `selection`, the same pair that deduplicates it locally. The preview,
/// kind, primary MIME type and size are not sent: the receiver derives
/// them from `payloads`, so a peer cannot make an entry look like
/// something it is not.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SyncRecord {
    /// BLAKE3 over every payload's MIME name and bytes ([`payload_hash`]).
    pub content_hash: String,
    /// Which selection the entry belongs to.
    pub selection: Selection,
    /// Best-effort source application name.
    pub source_app: Option<String>,
    /// Unix timestamp of the first capture, on the originating device.
    pub created_at: i64,
    /// Unix timestamp of the most recent capture.
    pub last_seen_at: i64,
    /// Pin state.
    pub pinned: bool,
    /// Tombstone: the entry was deleted. `payloads` is empty then.
    pub deleted: bool,
    /// Device that made this state.
    pub device_id: String,
    /// Lamport clock value of this state on `device_id`.
    pub lamport: i64,
    /// The entry's formats, in preference order; empty for a tombstone.
    pub payloads: Vec<MimePayload>,
}

/// Where a reader of the change feed left off: the local change counter
/// value of the last row it consumed (SYNC-04). Counter values are local,
/// so a cursor is only meaningful against the daemon that handed it out.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SyncCursor {
    /// Change counter value of the last consumed row.
    pub seq: i64,
}

/// Which entries a device shares (SYNC-03 selective sync). The default
/// shares everything that is not flagged sensitive; sensitive entries
/// never leave the device whatever this says.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SyncScope {
    /// Only pinned entries.
    #[serde(default)]
    pub pinned_only: bool,
    /// Only text-like entries (plain text, rich text, links, colours).
    #[serde(default)]
    pub text_only: bool,
}

impl SyncScope {
    /// Whether an entry of this kind and pin state is shared. Tombstones
    /// always are, so a deletion reaches peers that already hold the entry.
    pub fn includes(&self, kind: ContentKind, pinned: bool, deleted: bool) -> bool {
        if deleted {
            return true;
        }
        let text_like = matches!(
            kind,
            ContentKind::Text | ContentKind::RichText | ContentKind::Link | ContentKind::Color
        );
        (!self.pinned_only || pinned) && (!self.text_only || text_like)
    }
}

/// Last-writer-wins: does the state `(lamport, device_id)` supersede the
/// local one? Higher Lamport value wins; equal values fall back to the
/// device id so every device picks the same winner. A state never beats
/// itself, which is what makes applying the same record twice harmless.
pub fn lww_wins(remote: (i64, &str), local: (i64, &str)) -> bool {
    remote > local
}

/// The content hash an entry with these payloads has, the same way a
/// capture computes it: BLAKE3 over each payload's MIME name and bytes,
/// in order.
pub fn payload_hash(payloads: &[MimePayload]) -> String {
    let mut hasher = blake3::Hasher::new();
    for p in payloads {
        hasher.update(p.mime.as_bytes());
        hasher.update(&p.data);
    }
    hasher.finalize().to_hex().to_string()
}

/// A small helper used by future transports to report a disabled result.
pub fn disabled_result<T>() -> Result<T> {
    Err(crate::error::Error::Config(
        "sync transport is disabled in Panora v1".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn higher_lamport_wins_and_ties_break_on_device() {
        assert!(lww_wins((5, "a"), (4, "z")));
        assert!(!lww_wins((4, "z"), (5, "a")));
        assert!(lww_wins((5, "b"), (5, "a")));
        assert!(!lww_wins((5, "a"), (5, "b")));
        // Idempotent: the same state never replaces itself.
        assert!(!lww_wins((5, "a"), (5, "a")));
    }

    #[test]
    fn lww_is_a_total_order() {
        // For any two distinct states exactly one direction wins, so every
        // device converges on the same one whatever order records arrive in.
        let states = [(1, "a"), (1, "b"), (2, "a"), (3, "c"), (3, "a")];
        for x in states {
            for y in states {
                if x != y {
                    assert_ne!(lww_wins(x, y), lww_wins(y, x), "{x:?} vs {y:?}");
                }
            }
        }
    }

    #[test]
    fn scope_filters_live_entries_but_never_tombstones() {
        let all = SyncScope::default();
        assert!(all.includes(ContentKind::Image, false, false));
        let pinned = SyncScope {
            pinned_only: true,
            ..Default::default()
        };
        assert!(!pinned.includes(ContentKind::Text, false, false));
        assert!(pinned.includes(ContentKind::Text, true, false));
        let text = SyncScope {
            text_only: true,
            ..Default::default()
        };
        assert!(text.includes(ContentKind::Link, false, false));
        assert!(!text.includes(ContentKind::Image, true, false));
        assert!(!text.includes(ContentKind::FileList, false, false));
        assert!(text.includes(ContentKind::Image, false, true));
    }

    #[test]
    fn payload_hash_covers_mime_and_order() {
        let a = MimePayload::new("text/plain", "x");
        let b = MimePayload::new("text/html", "x");
        assert_ne!(
            payload_hash(std::slice::from_ref(&a)),
            payload_hash(std::slice::from_ref(&b))
        );
        assert_ne!(
            payload_hash(&[a.clone(), b.clone()]),
            payload_hash(&[b, a.clone()])
        );
        assert_eq!(payload_hash(std::slice::from_ref(&a)), payload_hash(&[a]));
    }
}
