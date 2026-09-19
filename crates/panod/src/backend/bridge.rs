// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! GNOME Shell bridge backend.
//!
//! Used on GNOME Wayland sessions whose Mutter has no data-control protocol
//! (GNOME < 48). Mutter refuses clipboard reads from unfocused clients, so
//! capture arrives through the Shell extension's D-Bus push (see
//! `gnome::GnomeBridge`) and this backend never watches or reads anything
//! itself. Recall and paste are delegated to the extension as well.

use async_trait::async_trait;
use panora_core::backend::{Capabilities, ClipboardBackend, ClipboardEvent};
use panora_core::error::{Error, Result};
use panora_core::model::{ClipboardData, MimePayload, Selection, TEXT_MIMES};
use std::sync::Mutex;
use tokio::sync::mpsc;

/// Backend that relies on the Shell extension for every clipboard operation.
#[derive(Default)]
pub struct GnomeBridgeBackend {
    /// Kept alive so the daemon's watch channel never reports "closed".
    watchers: Mutex<Vec<mpsc::Sender<ClipboardEvent>>>,
}

impl GnomeBridgeBackend {
    /// Construct the backend (always succeeds; the extension is contacted
    /// lazily).
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl ClipboardBackend for GnomeBridgeBackend {
    fn name(&self) -> &'static str {
        "gnome-bridge"
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            primary: false,
            images: true,
            // Mutter keeps clipboard content after the source exits.
            persist: true,
            synthetic_paste: true,
            // The Shell extension reports the focused window's app id.
            source_app: true,
            needs_bridge: true,
        }
    }

    async fn watch(&self, _selection: Selection) -> Result<mpsc::Receiver<ClipboardEvent>> {
        let (sender, receiver) = mpsc::channel(1);
        if let Ok(mut watchers) = self.watchers.lock() {
            watchers.push(sender);
        }
        Ok(receiver)
    }

    async fn read_targets(&self, _selection: Selection) -> Result<Vec<String>> {
        Err(Error::Backend(
            "GNOME bridge backend receives content from the Shell extension".into(),
        ))
    }

    async fn read(&self, _selection: Selection, _mime: &str) -> Result<Vec<u8>> {
        Err(Error::Backend(
            "GNOME bridge backend receives content from the Shell extension".into(),
        ))
    }

    async fn offer(&self, selection: Selection, data: ClipboardData) -> Result<()> {
        if selection != Selection::Clipboard {
            return Err(Error::Backend(
                "GNOME bridge backend only sets the clipboard selection".into(),
            ));
        }
        let payload = best_payload(&data.payloads)
            .ok_or_else(|| Error::Backend("nothing to offer".into()))?;
        crate::gnome::shell_set_clipboard(&payload.mime, &payload.data).await
    }

    async fn synthetic_paste(&self) -> Result<()> {
        crate::gnome::shell_paste().await
    }
}

/// St.Clipboard takes a single format, so pick the one applications expect:
/// images first, then file lists, then plain text, then whatever is left.
pub fn best_payload(payloads: &[MimePayload]) -> Option<&MimePayload> {
    payloads
        .iter()
        .find(|p| p.mime.starts_with("image/"))
        .or_else(|| payloads.iter().find(|p| p.mime == "text/uri-list"))
        .or_else(|| {
            TEXT_MIMES
                .iter()
                .find_map(|m| payloads.iter().find(|p| p.mime == *m))
        })
        .or_else(|| payloads.iter().find(|p| p.is_text()))
        .or_else(|| payloads.first())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_the_format_applications_expect() {
        let payloads = vec![
            MimePayload::new("text/html", "<b>a</b>"),
            MimePayload::new("text/plain", "a"),
        ];
        assert_eq!(best_payload(&payloads).unwrap().mime, "text/plain");
        let payloads = vec![
            MimePayload::new("text/plain", "x"),
            MimePayload::new("image/png", "png"),
        ];
        assert_eq!(best_payload(&payloads).unwrap().mime, "image/png");
        assert!(best_payload(&[]).is_none());
    }

    #[tokio::test]
    async fn watch_channel_stays_open() {
        let backend = GnomeBridgeBackend::new();
        let mut rx = backend.watch(Selection::Clipboard).await.unwrap();
        assert!(rx.try_recv().is_err());
        assert!(!rx.is_closed());
        assert!(backend.capabilities().needs_bridge);
    }
}
