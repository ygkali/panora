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
    /// Title of the focused window at the time of the copy, for the
    /// `excluded_window_titles` gate. Judged and dropped; never stored.
    pub source_title: Option<String>,
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
            source_title: None,
            kind: EventKind::Changed,
        }
    }

    /// The selection owner disappeared.
    pub fn owner_gone(selection: Selection) -> Self {
        Self {
            selection,
            offered_mimes: Vec::new(),
            source_app: None,
            source_title: None,
            kind: EventKind::OwnerGone,
        }
    }

    /// Attach the focused window's title (blank counts as unknown).
    pub fn with_title(mut self, title: Option<String>) -> Self {
        self.source_title = title.filter(|t| !t.trim().is_empty());
        self
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

    /// Release ownership of `selection`, best-effort (CAP-07: clear the
    /// live clipboard N seconds after a recall). Callers are expected to
    /// have already confirmed the selection still holds what they put
    /// there (e.g. via `read_targets`); a backend that cannot tell whether
    /// it is still the owner releases unconditionally rather than leaving
    /// stale content in place. Default implementation reports unsupported.
    async fn clear(&self, _selection: Selection) -> Result<()> {
        Err(crate::error::Error::Backend(
            "clearing the clipboard is not supported by this backend".into(),
        ))
    }
}

/// In-memory mock backend for tests and headless development.
///
/// Events are injected via `push_event`; `read` serves payloads from the
/// last `offer` call. This lets core logic be tested without a display.
#[derive(Debug, Default)]
pub struct MockBackend {
    /// Keyed by selection, like a real backend: offering on CLIPBOARD must
    /// never be visible when a test reads PRIMARY, and vice versa.
    offered: std::sync::Mutex<std::collections::HashMap<Selection, ClipboardData>>,
    /// One sender per watched selection, so a test can tell whether the
    /// daemon is currently listening to PRIMARY at all.
    senders: std::sync::Mutex<std::collections::HashMap<Selection, mpsc::Sender<ClipboardEvent>>>,
}

impl MockBackend {
    /// Create an empty mock.
    pub fn new() -> Self {
        Self::default()
    }

    /// Inject a synthetic change event (as if the user copied something).
    /// Dropped when nobody watches that selection, like a real display
    /// server that was never asked for the events.
    pub async fn push_event(&self, event: ClipboardEvent) {
        let tx = self.senders.lock().unwrap().get(&event.selection).cloned();
        if let Some(tx) = tx {
            let _ = tx.send(event).await;
        }
    }

    /// True while a live `watch` receiver exists for `selection`.
    pub fn watching(&self, selection: Selection) -> bool {
        self.senders
            .lock()
            .unwrap()
            .get(&selection)
            .is_some_and(|tx| !tx.is_closed())
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

    async fn watch(&self, selection: Selection) -> Result<mpsc::Receiver<ClipboardEvent>> {
        let (tx, rx) = mpsc::channel(64);
        self.senders.lock().unwrap().insert(selection, tx);
        Ok(rx)
    }

    async fn read_targets(&self, selection: Selection) -> Result<Vec<String>> {
        Ok(self
            .offered
            .lock()
            .unwrap()
            .get(&selection)
            .map(|d| d.offered_mimes.clone())
            .unwrap_or_default())
    }

    async fn read(&self, selection: Selection, mime: &str) -> Result<Vec<u8>> {
        self.offered
            .lock()
            .unwrap()
            .get(&selection)
            .and_then(|d| d.payloads.iter().find(|p| p.mime == mime))
            .map(|p| p.data.clone())
            .ok_or_else(|| crate::error::Error::Backend(format!("mime not offered: {mime}")))
    }

    async fn offer(&self, selection: Selection, data: ClipboardData) -> Result<()> {
        self.offered.lock().unwrap().insert(selection, data);
        Ok(())
    }

    async fn clear(&self, selection: Selection) -> Result<()> {
        self.offered.lock().unwrap().remove(&selection);
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

    #[tokio::test]
    async fn selections_are_independent() {
        let backend = MockBackend::new();
        backend
            .offer(
                Selection::Clipboard,
                ClipboardData {
                    selection: Selection::Clipboard,
                    payloads: vec![MimePayload::new("text/plain", "clip")],
                    offered_mimes: vec!["text/plain".into()],
                    source_app: None,
                },
            )
            .await
            .unwrap();
        backend
            .offer(
                Selection::Primary,
                ClipboardData {
                    selection: Selection::Primary,
                    payloads: vec![MimePayload::new("text/plain", "primary")],
                    offered_mimes: vec!["text/plain".into()],
                    source_app: None,
                },
            )
            .await
            .unwrap();

        assert_eq!(
            backend
                .read(Selection::Clipboard, "text/plain")
                .await
                .unwrap(),
            b"clip"
        );
        assert_eq!(
            backend
                .read(Selection::Primary, "text/plain")
                .await
                .unwrap(),
            b"primary"
        );

        backend.clear(Selection::Clipboard).await.unwrap();
        assert!(backend
            .read_targets(Selection::Clipboard)
            .await
            .unwrap()
            .is_empty());
        // Clearing CLIPBOARD must not touch PRIMARY.
        assert_eq!(
            backend
                .read(Selection::Primary, "text/plain")
                .await
                .unwrap(),
            b"primary"
        );
    }
}
