// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! Versioned JSON-lines IPC protocol shared by the daemon, the GUI and the CLI.
//!
//! One request per line, one response per line, over the user's private Unix
//! socket (`config::socket_path`). Every type here is the single source of
//! truth: the clients no longer carry their own copies, so the protocol cannot
//! drift between binaries built from the same tree.

use crate::error::{Error, Result};
use crate::model::{ContentKind, Entry, MimePayload};
use crate::storage::QueryFilter;
use serde::{Deserialize, Serialize};

#[cfg(unix)]
pub mod v3;

/// Protocol version reported by `Status`. Bump on incompatible changes.
///
/// v3 adds a binary framing (see [`v3`]) that carries large payloads as raw
/// bytes or passed file descriptors instead of base64-inflated JSON, plus
/// `Hello` negotiation and `Subscribe` push events. v2 clients (plain
/// JSON-lines, no preamble) are still served on the same socket for one
/// release: the daemon tells them apart by peeking the first byte of the
/// connection (`v3::MAGIC` vs. `{`).
pub const PROTOCOL_VERSION: u32 = 3;

/// Maximum accepted JSON-lines request frame size. This protects the daemon
/// from unbounded memory growth through the local IPC boundary.
pub const MAX_FRAME_BYTES: usize = 64 * 1024;
/// Maximum requests accepted on one long-lived client connection.
pub const MAX_REQUESTS_PER_CONNECTION: usize = 256;
/// Upper bound on one daemon reply accepted by clients. `Preview` carries
/// base64 payloads, so this is generous, but it must exist so that a
/// misbehaving daemon cannot make a client allocate without limit.
pub const MAX_RESPONSE_BYTES: u64 = 64 * 1024 * 1024;

/// JSON-lines/Unix-socket request.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "method", content = "params")]
pub enum Request {
    /// Return recent/search results.
    List(QueryRequest),
    /// Put an entry back on the clipboard, optionally pasting it into the
    /// focused window afterwards.
    Recall {
        /// Entry id.
        id: i64,
        /// Synthesize a paste keystroke after the clipboard is set.
        #[serde(default)]
        paste: bool,
        /// Offer only this format (e.g. `text/plain` to drop the HTML of a
        /// rich-text entry). `None` offers every stored format.
        #[serde(default)]
        mime: Option<String>,
    },
    /// Set pin state.
    Pin {
        /// Entry id.
        id: i64,
        /// New pin state.
        pinned: bool,
    },
    /// Delete one entry.
    Delete {
        /// Entry id.
        id: i64,
    },
    /// Delete all unpinned entries.
    Clear,
    /// Toggle private mode.
    SetPrivate {
        /// Whether recording should pause.
        enabled: bool,
    },
    /// Show/toggle the GUI popup.
    Toggle,
    /// Return health/capability info.
    Status,
    /// Return the decrypted payloads for one entry (preview or export).
    Preview {
        /// Entry id.
        id: i64,
        /// For image entries, return only a small PNG thumbnail (generated
        /// on capture, or on first request) instead of the full payloads.
        /// Entries without an image ignore the flag.
        #[serde(default)]
        thumbnail: bool,
    },
    /// Re-read `config.toml` and apply history/privacy limits live.
    ReloadConfig,
    /// Generate a new master key and reseal every stored preview and blob
    /// under it (SEC-01). Resumable: if a previous attempt was interrupted,
    /// this finishes it (the same pending key, picked back up from the
    /// keyring) instead of starting a fresh rotation. Blocks until done —
    /// there is no progress reporting, since it runs on the daemon's own
    /// task and reseals a whole history in one go.
    RotateKey,
    /// Engage the second-layer lock (SEC-02) by hand. Refused when no lock
    /// password is set.
    Lock,
    /// Disengage the second-layer lock.
    Unlock {
        /// Candidate password, checked against the stored verifier.
        password: String,
    },
    /// Set, change or remove the lock password. Changing or removing one
    /// that is already set requires `current_password` to verify first,
    /// regardless of whether the lock happens to be engaged right now.
    SetLockPassword {
        /// The new password, or `None` to remove the lock entirely.
        #[serde(default)]
        new_password: Option<String>,
        /// Required when a lock password is already set.
        #[serde(default)]
        current_password: Option<String>,
    },
    /// Panic wipe (SEC-03): hard-delete the whole history — pinned entries
    /// included, no undo — and retire the key that encrypted it in favour
    /// of a fresh one, so the old ciphertext is unrecoverable regardless of
    /// what happens to the deleted bytes on disk afterward. Not gated by
    /// the second-layer lock: a panic action has to work under duress, not
    /// only after unlocking first.
    Wipe,
    /// Bring back an entry deleted moments ago, while its tombstone is still
    /// inside the undo grace period.
    Restore {
        /// Entry id.
        id: i64,
    },
    /// Record content handed over by a client (`panora-cli store`) as if it
    /// had been copied: the same privacy gate applies, and `copy` also puts
    /// it on the clipboard.
    Store {
        /// Payloads in preference order; the first one is the primary format.
        payloads: Vec<MimePayload>,
        /// Application name the entry is attributed to (matched against the
        /// exclusion list like any capture).
        #[serde(default)]
        source_app: Option<String>,
        /// Also offer the content on the clipboard.
        #[serde(default)]
        copy: bool,
    },
    /// v3 preamble: negotiate the protocol version before any other
    /// request. v2 clients never send this (they have no preamble); a v3
    /// connection always opens with it. Ignored (but accepted) over the
    /// legacy JSON-lines transport, for tests that exercise both paths on
    /// the same socket.
    Hello {
        /// Highest protocol version the client understands.
        max_protocol: u32,
    },
    /// Turn this connection into a one-way event stream: the daemon stops
    /// expecting further requests on it and pushes an `Event::Changed` frame
    /// (starting with the current revision) every time `revision` changes,
    /// until the client disconnects. v3 framing only.
    Subscribe,
}

/// Serializable query parameters.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct QueryRequest {
    /// FTS search phrase.
    pub search: Option<String>,
    /// Content kind filter (`ContentKind::as_str` form).
    pub kind: Option<String>,
    /// Only pinned entries.
    pub pinned_only: bool,
    /// Page size (0 = default, capped at 500).
    pub limit: usize,
    /// Page offset.
    pub offset: usize,
}

impl From<QueryRequest> for QueryFilter {
    fn from(r: QueryRequest) -> Self {
        Self {
            search: r.search,
            kind: r.kind.as_deref().map(ContentKind::parse),
            pinned_only: r.pinned_only,
            limit: if r.limit == 0 { 50 } else { r.limit.min(500) },
            offset: r.offset,
            app: None,
            before: None,
            after: None,
            regex: None,
        }
    }
}

/// JSON-lines/Unix-socket response.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "ok", content = "data")]
pub enum Response {
    /// Successful response.
    #[serde(rename = "true")]
    Success(ResponseData),
    /// Error response safe to expose to clients.
    #[serde(rename = "false")]
    Failure {
        /// Human-readable reason.
        message: String,
    },
}

impl Response {
    /// Convert into a `Result`, mapping daemon failures to `Error::Ipc`.
    pub fn into_result(self) -> Result<ResponseData> {
        match self {
            Response::Success(data) => Ok(data),
            Response::Failure { message } => Err(Error::Ipc(message)),
        }
    }
}

/// Successful response payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ResponseData {
    /// History entries.
    Entries(Vec<Entry>),
    /// Number of affected entries.
    Count(usize),
    /// Daemon status.
    Status(StatusData),
    /// Empty successful response.
    Empty,
    /// Payloads for a selected entry.
    Payloads(Vec<MimePayload>),
    /// Outcome of `Recall`: the clipboard was set; `pasted` tells whether the
    /// requested paste keystroke could be delivered.
    Recalled {
        /// Paste keystroke delivered.
        pasted: bool,
    },
    /// Reply to `Hello`: the protocol version the daemon will use for the
    /// rest of this connection (`min(client max_protocol, daemon max)`).
    Hello {
        /// Negotiated protocol version.
        protocol: u32,
    },
}

/// A message the daemon pushes on its own initiative, over v3 framing, after
/// a `Subscribe` request. Not part of the `Response`/`ResponseData` pair
/// because it is not a reply to any one request.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event")]
pub enum Event {
    /// `revision` changed; clients re-fetch whatever view they hold instead
    /// of polling `Status`.
    #[serde(rename = "changed")]
    Changed {
        /// The new revision.
        revision: u64,
    },
}

/// Backend capability report exposed to clients.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CapabilityData {
    /// The PRIMARY selection can be recorded.
    pub primary: bool,
    /// Image payloads are supported.
    pub images: bool,
    /// The clipboard survives the source application exiting.
    pub persist: bool,
    /// A paste keystroke can be synthesized after recall.
    pub synthetic_paste: bool,
    /// The source application of a copy can be named, which is what the
    /// `excluded_apps` privacy list matches on. False on plain Wayland.
    #[serde(default)]
    pub source_app: bool,
    /// Capture depends on the GNOME Shell extension (GNOME without
    /// data-control).
    #[serde(default)]
    pub needs_bridge: bool,
}

/// Daemon status exposed to clients.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusData {
    /// Backend name.
    pub backend: String,
    /// Visible history count.
    pub entries: i64,
    /// Whether recording is paused.
    pub private_mode: bool,
    /// Whether a sync provider is active (always false in v1).
    pub sync_active: bool,
    /// Monotonic counter bumped on every history mutation; clients poll it
    /// to refresh their view cheaply.
    #[serde(default)]
    pub revision: u64,
    /// Daemon package version.
    #[serde(default)]
    pub version: String,
    /// IPC protocol version.
    #[serde(default)]
    pub protocol: u32,
    /// What the active backend can do.
    #[serde(default)]
    pub capabilities: CapabilityData,
    /// The session is locked (screensaver active); recording is paused
    /// until it unlocks, independently of private mode.
    #[serde(default)]
    pub locked: bool,
    /// The second-layer password lock (SEC-02) is currently engaged:
    /// `List` returns a count only, `Preview`/`Recall` are refused.
    #[serde(default)]
    pub app_locked: bool,
    /// Whether a lock password has been set at all, regardless of whether
    /// it is currently engaged.
    #[serde(default)]
    pub lock_password_set: bool,
    /// Conditions the user can act on. Each carries a stable code the
    /// clients key their guidance on (see `health`) and a plain English
    /// message for clients that know no better.
    #[serde(default)]
    pub health: Vec<HealthItem>,
}

/// One health finding of the daemon.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealthItem {
    /// Stable identifier, one of the constants in `health`.
    pub code: String,
    /// What is wrong and what to do about it, in English.
    pub message: String,
}

/// Health codes the daemon reports in `StatusData::health`.
pub mod health {
    /// Capture depends on the GNOME Shell extension and it does not own its
    /// bus name: nothing is recorded until it runs.
    pub const EXTENSION_MISSING: &str = "extension_missing";
    /// UUID of the GNOME Shell extension, as `gnome-extensions` knows it.
    pub const EXTENSION_UUID: &str = "panora@ygkali.github.io";
    /// A previous `RotateKey` (SEC-01) was interrupted before finishing;
    /// the pending key is still in the keyring and history is still fully
    /// readable (nothing is left half-resealed across a restart), but
    /// running `rotate-key` again is needed to retire it.
    pub const ROTATION_INCOMPLETE: &str = "rotation_incomplete";
}

/// Encode a value as one newline-terminated JSON frame.
pub fn encode<T: Serialize>(value: &T) -> serde_json::Result<Vec<u8>> {
    let mut bytes = serde_json::to_vec(value)?;
    bytes.push(b'\n');
    Ok(bytes)
}

/// Decode one JSON frame.
pub fn decode<'a, T: Deserialize<'a>>(line: &'a [u8]) -> serde_json::Result<T> {
    serde_json::from_slice(line)
}

/// Blocking client used by the GUI and the CLI.
#[cfg(unix)]
pub mod client {
    use super::{decode, encode, Request, Response, ResponseData, MAX_RESPONSE_BYTES};
    use crate::config::socket_path;
    use crate::error::{Error, Result};
    use std::io::{BufRead, BufReader, Read, Write};
    use std::os::unix::net::UnixStream;
    use std::time::Duration;

    /// IPC read/write timeout; the daemon is local, so this only guards hangs.
    pub const IPC_TIMEOUT: Duration = Duration::from_secs(5);

    /// Send one request and wait for its response.
    pub fn call(request: &Request) -> Result<ResponseData> {
        call_with_timeout(request, IPC_TIMEOUT)
    }

    /// Like `call`, but with a caller-chosen timeout instead of
    /// `IPC_TIMEOUT`. For requests the daemon can legitimately take a while
    /// to answer — `RotateKey` (SEC-01) reseals the whole history inline
    /// before replying — 5 seconds would fail a request that is still
    /// working, not hung.
    pub fn call_with_timeout(request: &Request, timeout: Duration) -> Result<ResponseData> {
        let stream = UnixStream::connect(socket_path())
            .map_err(|e| Error::Ipc(format!("daemon unavailable: {e}")))?;
        stream.set_read_timeout(Some(timeout))?;
        stream.set_write_timeout(Some(timeout))?;
        let mut writer = &stream;
        writer.write_all(&encode(request)?)?;
        let mut line = String::new();
        BufReader::new(&stream)
            .take(MAX_RESPONSE_BYTES)
            .read_line(&mut line)?;
        if line.is_empty() {
            return Err(Error::Ipc("daemon closed the connection".into()));
        }
        let response: Response = decode(line.trim_end_matches(['\r', '\n']).as_bytes())
            .map_err(|e| Error::Ipc(format!("invalid daemon response: {e}")))?;
        response.into_result()
    }

    /// A live `Subscribe` connection (STO-08, v3-only): the daemon pushes an
    /// [`super::Event`] every time its revision changes, starting with the
    /// current one, instead of the caller polling `Status`. Used by
    /// `panora-cli watch` and can back the GUI's live refresh too.
    pub struct Subscription {
        stream: UnixStream,
    }

    impl Subscription {
        /// Open a new subscription. Deliberately has no read timeout:
        /// callers block in `next` for as long as nothing changes.
        pub fn open() -> Result<Self> {
            let stream = UnixStream::connect(socket_path())
                .map_err(|e| Error::Ipc(format!("daemon unavailable: {e}")))?;
            stream.set_write_timeout(Some(IPC_TIMEOUT))?;
            super::v3::write_magic_blocking(&stream)?;
            super::v3::write_request_blocking(&stream, Request::Subscribe)?;
            Ok(Self { stream })
        }

        /// Block for the next event.
        pub fn next(&self) -> Result<super::Event> {
            super::v3::read_event_blocking(&self.stream)
        }
    }
}

/// Placeholder client for platforms without Unix sockets (never shipped;
/// keeps the clients compiling for cross-platform unit tests).
#[cfg(not(unix))]
pub mod client {
    use super::{Request, ResponseData};
    use crate::error::{Error, Result};

    /// Always fails: Panora's IPC needs a Unix socket.
    pub fn call(_request: &Request) -> Result<ResponseData> {
        Err(Error::Ipc(
            "daemon unavailable: Unix sockets unsupported".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_roundtrip() {
        let req = Request::List(QueryRequest {
            search: Some("merhaba".into()),
            limit: 20,
            ..Default::default()
        });
        let encoded = encode(&req).unwrap();
        let decoded: Request = decode(&encoded).unwrap();
        assert!(matches!(decoded, Request::List(_)));
    }

    #[test]
    fn recall_paste_defaults_to_false() {
        let decoded: Request = decode(br#"{"method":"Recall","params":{"id":3}}"#).unwrap();
        assert!(matches!(
            decoded,
            Request::Recall {
                id: 3,
                paste: false,
                mime: None
            }
        ));
    }

    #[test]
    fn frame_limits_are_bounded() {
        assert_eq!(MAX_FRAME_BYTES, 64 * 1024);
        assert_eq!(MAX_REQUESTS_PER_CONNECTION, 256);
    }

    #[test]
    fn reply_cap_carries_the_largest_allowed_payload() {
        // A Preview reply holds the payload base64-encoded (4/3 growth) plus
        // the JSON envelope; the config ceiling must never exceed what a
        // client is willing to read, or stored entries become unreadable.
        let encoded = (crate::config::MAX_MIME_BYTES_LIMIT as u64 * 4).div_ceil(3);
        assert!(encoded + 1024 * 1024 <= MAX_RESPONSE_BYTES);
    }

    #[test]
    fn query_limits_are_safe() {
        let filter: QueryFilter = QueryRequest {
            limit: 999_999,
            ..Default::default()
        }
        .into();
        assert_eq!(filter.limit, 500);
        let filter: QueryFilter = QueryRequest::default().into();
        assert_eq!(filter.limit, 50);
    }

    #[test]
    fn failure_maps_to_error() {
        let response = Response::Failure {
            message: "nope".into(),
        };
        assert!(matches!(response.into_result(), Err(Error::Ipc(m)) if m == "nope"));
    }

    #[test]
    fn status_tolerates_older_daemons() {
        let json = br#"{"ok":"true","data":{"Status":{"backend":"x11","entries":1,"private_mode":false,"sync_active":false}}}"#;
        let response: Response = decode(json).unwrap();
        let ResponseData::Status(status) = response.into_result().unwrap() else {
            panic!("expected status");
        };
        assert_eq!(status.revision, 0);
        assert!(!status.capabilities.primary);
    }
}
