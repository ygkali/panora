// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! Versioned IPC protocol definitions shared by GUI and CLI.

use panora_core::model::{ContentKind, Entry, MimePayload};
use panora_core::storage::QueryFilter;
use serde::{Deserialize, Serialize};

/// D-Bus interface name. Keep the numeric suffix for compatibility.
pub const DBUS_INTERFACE: &str = "io.panora.Pano1";
/// D-Bus well-known name.
pub const DBUS_NAME: &str = "io.panora.Pano1";
/// D-Bus object path.
pub const DBUS_PATH: &str = "/io/panora/Pano1";

/// Maximum accepted JSON-lines frame size. This protects the daemon from
/// unbounded memory growth through the local IPC boundary.
pub const MAX_FRAME_BYTES: usize = 64 * 1024;
/// Maximum requests accepted on one long-lived client connection.
pub const MAX_REQUESTS_PER_CONNECTION: usize = 256;

/// JSON-lines/Unix-socket request.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "method", content = "params")]
pub enum Request {
    /// Return recent/search results.
    List(QueryRequest),
    /// Put an entry back on the clipboard.
    Recall { id: i64 },
    /// Set pin state.
    Pin { id: i64, pinned: bool },
    /// Delete one entry.
    Delete { id: i64 },
    /// Delete all visible entries.
    Clear,
    /// Toggle private mode.
    SetPrivate { enabled: bool },
    /// Show/toggle the GUI popup.
    Toggle,
    /// Return health/capability info.
    Status,
    /// Return the encrypted payloads for a selected entry for UI preview/recall.
    Preview { id: i64 },
}

/// Serializable query parameters.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct QueryRequest {
    /// FTS search phrase.
    pub search: Option<String>,
    /// Content kind filter.
    pub kind: Option<String>,
    /// Only pinned entries.
    pub pinned_only: bool,
    /// Page size.
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
    Failure { message: String },
}

/// Successful response payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ResponseData {
    /// History entries.
    Entries(Vec<Entry>),
    /// Number of cleared entries.
    Count(usize),
    /// Status string.
    Status(StatusData),
    /// Empty successful response.
    Empty,
    /// Payloads for a selected entry; clients should decode only visible previews.
    Payloads(Vec<MimePayload>),
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
    /// Whether sync provider is active (false in v1.0).
    pub sync_active: bool,
}

/// Encode a request as one newline-terminated JSON frame.
pub fn encode<T: Serialize>(value: &T) -> serde_json::Result<Vec<u8>> {
    let mut bytes = serde_json::to_vec(value)?;
    bytes.push(b'\n');
    Ok(bytes)
}

/// Decode one JSON frame.
pub fn decode<'a, T: Deserialize<'a>>(line: &'a [u8]) -> serde_json::Result<T> {
    serde_json::from_slice(line)
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
    fn frame_limits_are_bounded() {
        assert_eq!(MAX_FRAME_BYTES, 64 * 1024);
        assert_eq!(MAX_REQUESTS_PER_CONNECTION, 256);
    }

    #[test]
    fn query_limits_are_safe() {
        let filter: QueryFilter = QueryRequest {
            limit: 999_999,
            ..Default::default()
        }
        .into();
        assert_eq!(filter.limit, 500);
    }
}
