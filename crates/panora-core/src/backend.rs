// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! Clipboard backend abstraction.
//!
//! Concrete backends (X11 via x11rb, Wayland via wl-clipboard-rs, GNOME
//! Shell extension bridge) live in the daemon crate. This module defines
//! the contract they implement, plus a mock backend used in tests.

use crate::error::Result;
use crate::model::{ClipboardData, Selection};
use async_trait::async_trait;
use tokio::sync::mpsc;

/// What happened to a selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EventKind {
    /// A new owner took the selection; `offered_mimes` lists its targets.
    #[default]
    Changed,
    /// The owner went away (application exit) and the selection is now
    /// empty. Backends that cannot persist content themselves report this so
    /// the daemon can re-offer the last entry (CLIPBOARD_MANAGER-like).
    OwnerGone,
}

/// Notification emitted by a backend when a selection changes.
#[derive(Debug, Clone)]
pub struct ClipboardEvent {
    /// Which selection changed.
    pub selection: Selection,
    /// MIME types advertised by the new owner (TARGETS). Always read
    /// before any payload so privacy flags can be honored (ADR 0003).
    pub offered_mimes: Vec<String>,
    /// Best-effort source application name.
    pub source_app: Option<String>,
    /// Change or loss of the owner.
    pub kind: EventKind,
}

impl ClipboardEvent {
    /// A new selection owner with the given TARGETS.
    pub fn changed(
        selection: Selection,
        offered_mimes: Vec<String>,
        source_app: Option<String>,
    ) -> Self {
        Self {
            selection,
            offered_mimes,
            source_app,
            kind: EventKind::Changed,
        }
    }

    /// The selection owner disappeared.
    pub fn owner_gone(selection: Selection) -> Self {
        Self {
            selection,
            offered_mimes: Vec::new(),
            source_app: None,
            kind: EventKind::OwnerGone,
        }
    }
}

/// Backend capability flags, reported at runtime so the daemon can
/// adapt (e.g. disable PRIMARY recording where unsupported).
#[derive(Debug, Clone, Copy, Default)]
pub struct Capabilities {
    /// Can watch the PRIMARY selection.
    pub primary: bool,
    /// Can receive image payloads.
    pub images: bool,
    /// Can persist clipboard content after the source app exits
    /// (CLIPBOARD_MANAGER / SAVE_TARGETS on X11).
    pub persist: bool,
    /// Can synthesize paste keystrokes (instant paste).
    pub synthetic_paste: bool,
    /// Can name the application a copy came from. The privacy engine's
    /// `excluded_apps` list matches on that name, so where this is false the
    /// list cannot fire and only the secret-flag MIME gate protects the user.
    /// The plain Wayland data-control protocols expose no client identity.
    pub source_app: bool,
    /// Capture depends on the GNOME Shell bridge extension pushing data
    /// (compositor without a data-control protocol). Native backends set
    /// this to false so bridge pushes are ignored instead of duplicated.
    pub needs_bridge: bool,
}

/// A clipboard capture/injection backend.
///
/// Contract:
/// - `watch` yields change events; the daemon then calls `read_targets`
///   and, only if the privacy engine allows, `read` for chosen MIMEs.
/// - `offer` puts data back on the clipboard (user picked a history item).
#[async_trait]
pub trait ClipboardBackend: Send + Sync {
    /// Human-readable backend name for logs ("x11", "wayland", "gnome-bridge").
    fn name(&self) -> &'static str;

    /// Runtime capability report.
    fn capabilities(&self) -> Capabilities;

    /// Start watching a selection. Returns a receiver of change events.
    /// Implementations are event-driven (XFIXES on X11, data-control on
    /// Wayland); they never poll or spawn helper processes.
    async fn watch(&self, selection: Selection) -> Result<mpsc::Receiver<ClipboardEvent>>;

    /// Read the current TARGETS (offered MIME list) without payloads.
    async fn read_targets(&self, selection: Selection) -> Result<Vec<String>>;

    /// Read a single MIME payload from the current clipboard owner.
    async fn read(&self, selection: Selection, mime: &str) -> Result<Vec<u8>>;

    /// Offer data on the clipboard, becoming the new owner. The data
    /// must remain available until another owner replaces it.
    async fn offer(&self, selection: Selection, data: ClipboardData) -> Result<()>;

    /// Best-effort: synthesize a paste keystroke into the focused window.
    /// Default implementation reports unsupported.
    async fn synthetic_paste(&self) -> Result<()> {
        Err(crate::error::Error::Backend(
            "synthetic paste not supported by this backend".into(),
        ))
    }
}

/// In-memory mock backend for tests and headless development.
///
/// Events are injected via `push_event`; `read` serves payloads from the
/// last `offer` call. This lets core logic be tested without a display.
#[derive(Debug, Default)]
pub struct MockBackend {
    offered: std::sync::Mutex<Option<ClipboardData>>,
    sender: std::sync::Mutex<Option<mpsc::Sender<ClipboardEvent>>>,
}

impl MockBackend {
    /// Create an empty mock.
    pub fn new() -> Self {
        Self::default()
    }

    /// Inject a synthetic change event (as if the user copied something).
    pub async fn push_event(&self, event: ClipboardEvent) {
        let tx = self.sender.lock().unwrap().clone();
        if let Some(tx) = tx {
            let _ = tx.send(event).await;
        }
    }
}

#[async_trait]
impl ClipboardBackend for MockBackend {
    fn name(&self) -> &'static str {
        "mock"
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            primary: true,
            images: true,
            persist: true,
            synthetic_paste: false,
            source_app: true,
            needs_bridge: false,
        }
    }

    async fn watch(&self, _selection: Selection) -> Result<mpsc::Receiver<ClipboardEvent>> {
        let (tx, rx) = mpsc::channel(64);
        *self.sender.lock().unwrap() = Some(tx);
        Ok(rx)
    }

    async fn read_targets(&self, _selection: Selection) -> Result<Vec<String>> {
        Ok(self
            .offered
            .lock()
            .unwrap()
            .as_ref()
            .map(|d| d.offered_mimes.clone())
            .unwrap_or_default())
    }

    async fn read(&self, _selection: Selection, mime: &str) -> Result<Vec<u8>> {
        self.offered
            .lock()
            .unwrap()
            .as_ref()
            .and_then(|d| d.payloads.iter().find(|p| p.mime == mime))
            .map(|p| p.data.clone())
            .ok_or_else(|| crate::error::Error::Backend(format!("mime not offered: {mime}")))
    }

    async fn offer(&self, _selection: Selection, data: ClipboardData) -> Result<()> {
        *self.offered.lock().unwrap() = Some(data);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::MimePayload;

    #[tokio::test]
    async fn mock_roundtrip() {
        let backend = MockBackend::new();
        let mut rx = backend.watch(Selection::Clipboard).await.unwrap();

        let data = ClipboardData {
            selection: Selection::Clipboard,
            payloads: vec![MimePayload {
                mime: "text/plain".into(),
                data: b"hello".to_vec(),
            }],
            offered_mimes: vec!["text/plain".into()],
            source_app: Some("test".into()),
        };
        backend.offer(Selection::Clipboard, data).await.unwrap();

        backend
            .push_event(ClipboardEvent::changed(
                Selection::Clipboard,
                vec!["text/plain".into()],
                Some("test".into()),
            ))
            .await;

        let ev = rx.recv().await.unwrap();
        assert_eq!(ev.selection, Selection::Clipboard);
        assert_eq!(ev.offered_mimes, vec!["text/plain"]);
        assert_eq!(ev.kind, EventKind::Changed);
        assert_eq!(
            ClipboardEvent::owner_gone(Selection::Primary).kind,
            EventKind::OwnerGone
        );

        let targets = backend.read_targets(Selection::Clipboard).await.unwrap();
        assert_eq!(targets, vec!["text/plain"]);

        let payload = backend
            .read(Selection::Clipboard, "text/plain")
            .await
            .unwrap();
        assert_eq!(payload, b"hello");

        assert!(backend
            .read(Selection::Clipboard, "image/png")
            .await
            .is_err());
    }
}
