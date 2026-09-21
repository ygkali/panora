// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! Public, versioned D-Bus API mirroring the IPC (INT-05).
//!
//! `io.github.ygkali.Panora1` on the session bus, at
//! `/io/github/ygkali/Panora1`. A thin translation layer, not a second
//! implementation: every method call becomes exactly the same
//! `panora_core::ipc::Request` the Unix socket already accepts, sent
//! through `panora_core::ipc::client::call` — panod is its own IPC client
//! here, the same way `panora-cli` is, so there is exactly one place the
//! privacy gates, encryption and dedup logic live. The Unix socket stays
//! the internal, primary protocol; this exists for third-party
//! integrations that would rather speak D-Bus (Waybar modules, shell
//! scripts via `gdbus`/`dbus-send`, …) than open a Unix socket themselves.
//! `docs/dbus-api.md` documents the interface for that audience.

use panora_core::error::{Error, Result};
use panora_core::ipc::{client, Event, QueryRequest, Request, ResponseData};
use panora_core::model::Entry;
use std::collections::HashMap;
use tracing::warn;
use zbus::zvariant::{OwnedValue, Value};

/// Bus name this service owns.
pub const BUS_NAME: &str = "io.github.ygkali.Panora1";
/// Object path the interface is served at.
pub const OBJECT_PATH: &str = "/io/github/ygkali/Panora1";

/// Run a blocking IPC call on a blocking-pool thread and translate its
/// error into one `zbus::fdo::Error` shape callers can rely on.
async fn call(request: Request) -> zbus::fdo::Result<ResponseData> {
    tokio::task::spawn_blocking(move || client::call(&request))
        .await
        .map_err(|e| zbus::fdo::Error::Failed(format!("IPC task panicked: {e}")))?
        .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))
}

fn unexpected(data: ResponseData) -> zbus::fdo::Error {
    zbus::fdo::Error::Failed(format!("unexpected daemon reply: {data:?}"))
}

/// The interface object. Stateless: every call re-derives everything it
/// needs from the request/reply, so there is nothing to keep in sync here.
pub struct PublicApi;

fn get_str(filters: &HashMap<String, OwnedValue>, key: &str) -> Option<String> {
    filters
        .get(key)
        .and_then(|v| TryInto::<&str>::try_into(v).ok())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn get_bool(filters: &HashMap<String, OwnedValue>, key: &str) -> bool {
    filters
        .get(key)
        .and_then(|v| TryInto::<bool>::try_into(v).ok())
        .unwrap_or(false)
}

fn get_u32(filters: &HashMap<String, OwnedValue>, key: &str) -> Option<u32> {
    filters
        .get(key)
        .and_then(|v| TryInto::<u32>::try_into(v).ok())
}

fn query_from_filters(filters: &HashMap<String, OwnedValue>) -> QueryRequest {
    QueryRequest {
        search: get_str(filters, "search"),
        kind: get_str(filters, "kind"),
        pinned_only: get_bool(filters, "pinned_only"),
        limit: get_u32(filters, "limit").unwrap_or(0) as usize,
        offset: get_u32(filters, "offset").unwrap_or(0) as usize,
    }
}

fn entry_to_dict(entry: &Entry) -> HashMap<String, OwnedValue> {
    let mut dict = HashMap::new();
    dict.insert("id".into(), Value::from(entry.id).try_into().unwrap());
    dict.insert(
        "content_hash".into(),
        Value::from(entry.content_hash.as_str()).try_into().unwrap(),
    );
    dict.insert(
        "preview".into(),
        Value::from(entry.preview.as_str()).try_into().unwrap(),
    );
    dict.insert(
        "kind".into(),
        Value::from(entry.kind.as_str()).try_into().unwrap(),
    );
    dict.insert(
        "primary_mime".into(),
        Value::from(entry.primary_mime.as_str()).try_into().unwrap(),
    );
    dict.insert(
        "size_bytes".into(),
        Value::from(entry.size_bytes).try_into().unwrap(),
    );
    dict.insert(
        "source_app".into(),
        Value::from(entry.source_app.as_deref().unwrap_or(""))
            .try_into()
            .unwrap(),
    );
    dict.insert(
        "created_at".into(),
        Value::from(entry.created_at).try_into().unwrap(),
    );
    dict.insert(
        "last_seen_at".into(),
        Value::from(entry.last_seen_at).try_into().unwrap(),
    );
    dict.insert(
        "pinned".into(),
        Value::from(entry.pinned).try_into().unwrap(),
    );
    dict.insert(
        "sensitive".into(),
        Value::from(entry.sensitive).try_into().unwrap(),
    );
    dict.insert(
        "selection".into(),
        Value::from(entry.selection.as_str()).try_into().unwrap(),
    );
    dict
}

fn status_to_dict(status: &panora_core::ipc::StatusData) -> HashMap<String, OwnedValue> {
    let mut dict = HashMap::new();
    dict.insert(
        "backend".into(),
        Value::from(status.backend.as_str()).try_into().unwrap(),
    );
    dict.insert(
        "entries".into(),
        Value::from(status.entries).try_into().unwrap(),
    );
    dict.insert(
        "private_mode".into(),
        Value::from(status.private_mode).try_into().unwrap(),
    );
    dict.insert(
        "sync_active".into(),
        Value::from(status.sync_active).try_into().unwrap(),
    );
    dict.insert(
        "revision".into(),
        Value::from(status.revision).try_into().unwrap(),
    );
    dict.insert(
        "version".into(),
        Value::from(status.version.as_str()).try_into().unwrap(),
    );
    dict.insert(
        "protocol".into(),
        Value::from(status.protocol).try_into().unwrap(),
    );
    dict.insert(
        "locked".into(),
        Value::from(status.locked).try_into().unwrap(),
    );
    dict.insert(
        "app_locked".into(),
        Value::from(status.app_locked).try_into().unwrap(),
    );
    dict.insert(
        "lock_password_set".into(),
        Value::from(status.lock_password_set).try_into().unwrap(),
    );
    dict
}

#[zbus::interface(name = "io.github.ygkali.Panora1")]
impl PublicApi {
    /// Filters: `search` (s), `kind` (s), `pinned_only` (b), `limit` (u),
    /// `offset` (u) — all optional, same meaning as the CLI's flags of the
    /// same names. While the second-layer lock (SEC-02) is engaged this
    /// returns an empty list, the same as `panora-cli list` degrading to a
    /// count over the Unix socket, since a dict-array reply has nowhere to
    /// carry "count only" — call `Status` for the count instead.
    async fn list(
        &self,
        filters: HashMap<String, OwnedValue>,
    ) -> zbus::fdo::Result<Vec<HashMap<String, OwnedValue>>> {
        match call(Request::List(query_from_filters(&filters))).await? {
            ResponseData::Entries(entries) => Ok(entries.iter().map(entry_to_dict).collect()),
            ResponseData::Count(_) => Ok(Vec::new()),
            other => Err(unexpected(other)),
        }
    }

    /// Puts an entry back on the clipboard. `mime` empty means every
    /// stored format; returns whether a paste keystroke was delivered
    /// (meaningless when `paste` is false).
    async fn recall(&self, id: i64, paste: bool, mime: String) -> zbus::fdo::Result<bool> {
        let mime = (!mime.is_empty()).then_some(mime);
        match call(Request::Recall { id, paste, mime }).await? {
            ResponseData::Recalled { pasted } => Ok(pasted),
            other => Err(unexpected(other)),
        }
    }

    async fn pin(&self, id: i64, pinned: bool) -> zbus::fdo::Result<()> {
        call(Request::Pin { id, pinned }).await?;
        Ok(())
    }

    async fn delete(&self, id: i64) -> zbus::fdo::Result<()> {
        call(Request::Delete { id }).await?;
        Ok(())
    }

    /// Deletes every unpinned entry; returns how many were removed.
    async fn clear(&self) -> zbus::fdo::Result<u32> {
        match call(Request::Clear).await? {
            ResponseData::Count(n) => Ok(n as u32),
            other => Err(unexpected(other)),
        }
    }

    #[zbus(name = "SetPrivate")]
    async fn set_private(&self, enabled: bool) -> zbus::fdo::Result<()> {
        call(Request::SetPrivate { enabled }).await?;
        Ok(())
    }

    /// Same fields as `panora-cli status`, as a dict: `backend` (s),
    /// `entries` (x), `private_mode` (b), `sync_active` (b), `revision`
    /// (t), `version` (s), `protocol` (u), `locked` (b), `app_locked` (b),
    /// `lock_password_set` (b).
    async fn status(&self) -> zbus::fdo::Result<HashMap<String, OwnedValue>> {
        match call(Request::Status).await? {
            ResponseData::Status(status) => Ok(status_to_dict(&status)),
            other => Err(unexpected(other)),
        }
    }

    /// Emitted every time the history changes, carrying the new revision
    /// — the D-Bus equivalent of the IPC's `Subscribe` event stream
    /// (STO-08), for a client that would rather watch a signal than hold
    /// a socket open.
    #[zbus(signal)]
    pub async fn changed(
        signal_emitter: &zbus::object_server::SignalEmitter<'_>,
        revision: u64,
    ) -> zbus::Result<()>;
}

/// Serve the public API on the session bus and forward every `Subscribe`
/// event (STO-08) as a `Changed` signal, until the connection or the
/// subscription ends. Runs for the daemon's whole lifetime; errors here
/// are logged and the service is simply unavailable, the same as the
/// GNOME bridge when it cannot start (INT-05 is a convenience surface,
/// not something capture depends on).
pub async fn run() -> Result<()> {
    let connection = zbus::connection::Builder::session()
        .map_err(|e| Error::Backend(format!("D-Bus session connection: {e}")))?
        .name(BUS_NAME)
        .map_err(|e| Error::Backend(format!("D-Bus name request: {e}")))?
        .serve_at(OBJECT_PATH, PublicApi)
        .map_err(|e| Error::Backend(format!("D-Bus object registration: {e}")))?
        .build()
        .await
        .map_err(|e| Error::Backend(format!("D-Bus connection: {e}")))?;

    let iface_ref = connection
        .object_server()
        .interface::<_, PublicApi>(OBJECT_PATH)
        .await
        .map_err(|e| Error::Backend(format!("D-Bus interface lookup: {e}")))?;

    // Subscribe is blocking I/O (STO-08's client is a plain std socket);
    // run it on a blocking-pool thread and forward each event over a
    // channel to the async task that actually emits the signal.
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Event>(8);
    tokio::task::spawn_blocking(move || {
        let subscription = match client::Subscription::open() {
            Ok(subscription) => subscription,
            Err(e) => {
                warn!(error = %e, "D-Bus API: could not open the Subscribe connection");
                return;
            }
        };
        loop {
            match subscription.next() {
                Ok(event) => {
                    if tx.blocking_send(event).is_err() {
                        return; // receiver gone: connection/task ended
                    }
                }
                Err(e) => {
                    warn!(error = %e, "D-Bus API: Subscribe connection ended");
                    return;
                }
            }
        }
    });

    while let Some(Event::Changed { revision }) = rx.recv().await {
        let emitter = iface_ref.signal_emitter();
        if let Err(e) = PublicApi::changed(emitter, revision).await {
            warn!(error = %e, "D-Bus API: could not emit Changed");
        }
    }
    Ok(())
}
