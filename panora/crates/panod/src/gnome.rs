// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! GNOME Shell bridge boundary.

use panora_core::model::{ClipboardData, MimePayload, Selection};
use std::sync::Mutex;
use tokio::sync::mpsc;

/// GNOME bridge bus name.
pub const BUS_NAME: &str = "io.panora.GnomeBridge1";
/// GNOME bridge object path.
pub const OBJECT_PATH: &str = "/io/panora/GnomeBridge1";

/// Small D-Bus endpoint used by the GNOME Shell extension.
pub struct GnomeBridge {
    sender: Mutex<mpsc::Sender<ClipboardData>>,
}

impl GnomeBridge {
    /// Create a bridge and bounded event receiver.
    pub fn new(capacity: usize) -> (Self, mpsc::Receiver<ClipboardData>) {
        let (sender, receiver) = mpsc::channel(capacity);
        (
            Self {
                sender: Mutex::new(sender),
            },
            receiver,
        )
    }
}

#[zbus::interface(name = "io.panora.GnomeBridge1")]
impl GnomeBridge {
    /// Receive a clipboard payload from GNOME Shell.
    async fn push(
        &self,
        mimes: Vec<String>,
        mime: String,
        bytes: Vec<u8>,
        source_app: String,
    ) -> zbus::fdo::Result<()> {
        let data = ClipboardData {
            selection: Selection::Clipboard,
            offered_mimes: if mimes.is_empty() {
                vec![mime.clone()]
            } else {
                mimes
            },
            payloads: vec![MimePayload { mime, data: bytes }],
            source_app: if source_app.trim().is_empty() {
                None
            } else {
                Some(source_app)
            },
        };
        let sender = self
            .sender
            .lock()
            .map_err(|_| zbus::fdo::Error::Failed("GNOME bridge channel lock poisoned".into()))?
            .clone();
        sender
            .send(data)
            .await
            .map_err(|_| zbus::fdo::Error::Failed("daemon bridge receiver stopped".into()))
    }
}
