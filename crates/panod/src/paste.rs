// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! Synthetic Ctrl+V on Wayland.
//!
//! X11 uses XTEST inside the X11 backend. Wayland has no portable input
//! injection, so this tries, in order: the Panora GNOME Shell extension
//! (Clutter virtual keyboard), `wtype` (wlroots virtual-keyboard protocol)
//! and `ydotool` (uinput, needs its daemon). Everything else reports that
//! instant paste is unavailable; the clipboard content is set regardless.

use panora_core::error::{Error, Result};
use tracing::debug;

/// Helper commands tried after the Shell extension, with their arguments.
const HELPERS: &[(&str, &[&str])] = &[
    ("wtype", &["-M", "ctrl", "-k", "v", "-m", "ctrl"]),
    ("ydotool", &["key", "29:1", "47:1", "47:0", "29:0"]),
];

/// Best-effort paste keystroke for the current Wayland session.
pub async fn wayland_paste() -> Result<()> {
    match crate::gnome::shell_paste().await {
        Ok(()) => return Ok(()),
        Err(e) => debug!(error = %e, "GNOME Shell paste unavailable"),
    }
    for (command, args) in HELPERS {
        match tokio::process::Command::new(command)
            .args(*args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await
        {
            Ok(status) if status.success() => return Ok(()),
            Ok(status) => debug!(command, ?status, "paste helper failed"),
            Err(e) => debug!(command, error = %e, "paste helper unavailable"),
        }
    }
    Err(Error::Backend(
        "no paste helper available (GNOME extension, wtype or ydotool)".into(),
    ))
}
