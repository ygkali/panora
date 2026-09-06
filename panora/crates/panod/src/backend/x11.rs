// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! X11 clipboard backend.

use async_trait::async_trait;
use panora_core::backend::{Capabilities, ClipboardBackend, ClipboardEvent};
use panora_core::error::{Error, Result};
use panora_core::model::{ClipboardData, Selection};
use std::process::{Command, Stdio};
use tokio::sync::mpsc;
use tokio::time::{sleep, Duration};
use tracing::debug;
use x11rb::connection::Connection as _;
use x11rb::protocol::xproto::{Atom, AtomEnum, ConnectionExt as _, Window};
use x11rb::rust_connection::RustConnection;

/// How far up the window tree to look for a WM_CLASS before giving up.
const WM_CLASS_DEPTH: usize = 8;

/// X11 backend using the standard xclip TARGETS protocol.
#[derive(Debug, Clone, Copy, Default)]
pub struct X11Backend;

impl X11Backend {
    /// Verify that the current X11 clipboard can be queried.
    pub fn connect() -> Result<Self> {
        if std::env::var_os("DISPLAY").is_none() {
            return Err(Error::Backend("DISPLAY is not set".into()));
        }
        if Command::new("xclip").arg("-version").output().is_err() {
            return Err(Error::Backend("xclip is not installed".into()));
        }
        Ok(Self)
    }
}

#[async_trait]
impl ClipboardBackend for X11Backend {
    fn name(&self) -> &'static str {
        "x11"
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            primary: true,
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
                let snapshot = tokio::task::spawn_blocking(move || snapshot(selection))
                    .await
                    .ok()
                    .and_then(std::result::Result::ok)
                    .unwrap_or_default();
                let target_text = snapshot.0;
                if !target_text.is_empty() && snapshot.1 != last {
                    let offered_mimes = target_text
                        .lines()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(ToOwned::to_owned)
                        .collect::<Vec<_>>();
                    last = snapshot.1;
                    // Only resolved when the clipboard actually changed, so
                    // this costs one X11 round trip per copy, not per poll.
                    let source_app = tokio::task::spawn_blocking(focused_app)
                        .await
                        .ok()
                        .flatten();
                    debug!(?source_app, "x11 clipboard change");
                    if sender
                        .send(ClipboardEvent {
                            selection,
                            offered_mimes,
                            source_app,
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

    async fn read_targets(&self, selection: Selection) -> Result<Vec<String>> {
        let output = tokio::task::spawn_blocking(move || targets(selection))
            .await
            .map_err(|e| Error::Backend(e.to_string()))??;
        Ok(output
            .lines()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToOwned::to_owned)
            .collect())
    }

    async fn read(&self, selection: Selection, mime: &str) -> Result<Vec<u8>> {
        let selection = selection_arg(selection);
        let mime = mime.to_string();
        tokio::task::spawn_blocking(move || {
            let output = Command::new("xclip")
                .args(["-selection", selection, "-out", "-t", &mime])
                .output()
                .map_err(|e| Error::Backend(e.to_string()))?;
            if !output.status.success() {
                return Err(Error::Backend(format!("xclip failed for MIME {mime}")));
            }
            Ok(output.stdout)
        })
        .await
        .map_err(|e| Error::Backend(e.to_string()))?
    }

    async fn offer(&self, selection: Selection, data: ClipboardData) -> Result<()> {
        let selection = selection_arg(selection).to_string();
        for payload in data.payloads {
            let mut child = Command::new("xclip")
                .args(["-selection", &selection, "-in", "-t", &payload.mime])
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
                    "xclip offer failed for {}",
                    payload.mime
                )));
            }
        }
        Ok(())
    }
}

/// Best-effort name of the application the copy came from.
///
/// This is what the privacy engine's `excluded_apps` list matches against, so
/// without it the KeePassXC/Bitwarden/1Password exclusions never fire on X11.
///
/// X11 offers no reliable way to name the *selection owner*: toolkits hand
/// ownership to a hidden proxy window that carries no WM_CLASS. The focused
/// top-level is the same heuristic the GNOME bridge uses and matches what the
/// user was actually working in when they pressed Ctrl+C. Every failure path
/// returns None: a missing source app must never block a capture.
fn focused_app() -> Option<String> {
    let (conn, screen_num) = x11rb::connect(None).ok()?;
    let root = conn.setup().roots.get(screen_num)?.root;

    let window = intern(&conn, b"_NET_ACTIVE_WINDOW")
        .and_then(|atom| active_window(&conn, root, atom))
        .or_else(|| conn.get_input_focus().ok()?.reply().ok().map(|r| r.focus))?;

    // WM_CLASS lives on the top-level, but focus usually sits on a child.
    let mut window = window;
    for _ in 0..WM_CLASS_DEPTH {
        if let Some(class) = wm_class(&conn, window) {
            return Some(class);
        }
        let tree = conn.query_tree(window).ok()?.reply().ok()?;
        if tree.parent == x11rb::NONE || tree.parent == window || window == root {
            break;
        }
        window = tree.parent;
    }
    None
}

fn intern(conn: &RustConnection, name: &[u8]) -> Option<Atom> {
    let atom = conn.intern_atom(true, name).ok()?.reply().ok()?.atom;
    (atom != x11rb::NONE).then_some(atom)
}

fn active_window(conn: &RustConnection, root: Window, atom: Atom) -> Option<Window> {
    let reply = conn
        .get_property(false, root, atom, AtomEnum::WINDOW, 0, 1)
        .ok()?
        .reply()
        .ok()?;
    let id = reply.value32()?.next()?;
    (id != x11rb::NONE).then_some(id)
}

/// WM_CLASS is "instance\0class\0"; the class half is the stable app name.
fn wm_class(conn: &RustConnection, window: Window) -> Option<String> {
    let reply = conn
        .get_property(false, window, AtomEnum::WM_CLASS, AtomEnum::STRING, 0, 256)
        .ok()?
        .reply()
        .ok()?;
    if reply.value.is_empty() {
        return None;
    }
    let text = String::from_utf8_lossy(&reply.value);
    let mut parts = text.split('\0').filter(|part| !part.is_empty());
    let instance = parts.next()?;
    Some(parts.next().unwrap_or(instance).to_string())
}

fn selection_arg(selection: Selection) -> &'static str {
    match selection {
        Selection::Clipboard => "clipboard",
        Selection::Primary => "primary",
    }
}

fn snapshot(selection: Selection) -> Result<(String, String)> {
    let target_text = targets(selection)?;
    if target_text.is_empty() {
        return Ok((String::new(), String::new()));
    }
    // TARGETS often stays constant for consecutive text copies. X11's
    // TIMESTAMP target changes with the clipboard owner and avoids losing
    // events without reading large image payloads on every poll.
    let timestamp = read_target(selection, "TIMESTAMP").unwrap_or_default();
    let fingerprint = if timestamp.is_empty() {
        format!(
            "{}:{}",
            target_text,
            blake3::hash(target_text.as_bytes()).to_hex()
        )
    } else {
        blake3::hash(&timestamp).to_hex().to_string()
    };
    Ok((target_text, fingerprint))
}

fn targets(selection: Selection) -> Result<String> {
    let selection = selection_arg(selection);
    let output = Command::new("xclip")
        .args(["-selection", selection, "-target", "TARGETS", "-out"])
        .output()
        .map_err(|e| Error::Backend(e.to_string()))?;
    if !output.status.success() {
        return Ok(String::new());
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn read_target(selection: Selection, mime: &str) -> Result<Vec<u8>> {
    let output = Command::new("xclip")
        .args(["-selection", selection_arg(selection), "-out", "-t", mime])
        .output()
        .map_err(|e| Error::Backend(e.to_string()))?;
    if !output.status.success() {
        return Err(Error::Backend(format!(
            "xclip target read failed for {mime}"
        )));
    }
    Ok(output.stdout)
}
