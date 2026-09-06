// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! Wayland clipboard backend.

use async_trait::async_trait;
use panora_core::backend::{Capabilities, ClipboardBackend, ClipboardEvent};
use panora_core::error::{Error, Result};
use panora_core::model::{ClipboardData, Selection};
use std::process::{Command, Stdio};
use tokio::sync::mpsc;
use tokio::time::{sleep, Duration};

/// Wayland backend using wl-clipboard's data-control integration.
#[derive(Debug, Clone, Copy, Default)]
pub struct WaylandBackend;

impl WaylandBackend {
    /// Verify that wl-paste/wl-copy are available.
    pub fn connect() -> Result<Self> {
        if std::env::var_os("WAYLAND_DISPLAY").is_none() {
            return Err(Error::Backend("WAYLAND_DISPLAY is not set".into()));
        }
        for command in ["wl-paste", "wl-copy"] {
            if Command::new(command).arg("--version").output().is_err() {
                return Err(Error::Backend(format!("{command} is not installed")));
            }
        }
        Ok(Self)
    }
}

#[async_trait]
impl ClipboardBackend for WaylandBackend {
    fn name(&self) -> &'static str {
        "wayland"
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            primary: false,
            images: true,
            persist: false,
            synthetic_paste: false,
        }
    }

    async fn watch(&self, selection: Selection) -> Result<mpsc::Receiver<ClipboardEvent>> {
        let (sender, receiver) = mpsc::channel(64);
        tokio::spawn(async move {
            let mut last = String::new();
            loop {
                let current = tokio::task::spawn_blocking(types)
                    .await
                    .ok()
                    .and_then(std::result::Result::ok)
                    .unwrap_or_default();
                if !current.is_empty() && current != last {
                    let offered_mimes = current
                        .lines()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(ToOwned::to_owned)
                        .collect::<Vec<_>>();
                    last = current;
                    if sender
                        .send(ClipboardEvent {
                            selection,
                            // Wayland's data-control protocol deliberately
                            // exposes no client identity, so the privacy
                            // engine's excluded_apps list cannot match here.
                            // The MIME secret-flag gate still applies, and on
                            // GNOME the Shell extension supplies the real app
                            // id. Documented in docs/protocol-matrix.md.
                            source_app: None,
                            offered_mimes,
                        })
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                sleep(Duration::from_millis(180)).await;
            }
        });
        Ok(receiver)
    }

    async fn read_targets(&self, _selection: Selection) -> Result<Vec<String>> {
        let text = tokio::task::spawn_blocking(types)
            .await
            .map_err(|e| Error::Backend(e.to_string()))??;
        Ok(text
            .lines()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToOwned::to_owned)
            .collect())
    }

    async fn read(&self, _selection: Selection, mime: &str) -> Result<Vec<u8>> {
        let mime = mime.to_string();
        tokio::task::spawn_blocking(move || {
            let output = Command::new("wl-paste")
                .args(["--no-newline", "--type", &mime])
                .output()
                .map_err(|e| Error::Backend(e.to_string()))?;
            if !output.status.success() {
                return Err(Error::Backend(format!("wl-paste failed for MIME {mime}")));
            }
            Ok(output.stdout)
        })
        .await
        .map_err(|e| Error::Backend(e.to_string()))?
    }

    async fn offer(&self, _selection: Selection, data: ClipboardData) -> Result<()> {
        for payload in data.payloads {
            let mut child = Command::new("wl-copy")
                .args(["--type", &payload.mime])
                .stdin(Stdio::piped())
                .spawn()
                .map_err(|e| Error::Backend(e.to_string()))?;
            if let Some(stdin) = child.stdin.as_mut() {
                use std::io::Write;
                stdin.write_all(&payload.data)?;
            }
            let status = child.wait().map_err(|e| Error::Backend(e.to_string()))?;
            if !status.success() {
                return Err(Error::Backend(format!(
                    "wl-copy failed for {}",
                    payload.mime
                )));
            }
        }
        Ok(())
    }
}

fn types() -> Result<String> {
    let output = Command::new("wl-paste")
        .args(["--list-types"])
        .output()
        .map_err(|e| Error::Backend(e.to_string()))?;
    if !output.status.success() {
        return Ok(String::new());
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}
