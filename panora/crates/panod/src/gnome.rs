// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! GNOME Shell integration over the session bus.
//!
//! Two directions:
//! * The Shell extension pushes clipboard changes *to* panod through the
//!   `io.panora.GnomeBridge1` service exported here (needed on GNOME
//!   versions without a data-control protocol, where Mutter denies other
//!   clients clipboard reads).
//! * panod asks the extension (`io.panora.GnomeShell1`) to set the
//!   clipboard or to synthesize a paste keystroke, and asks the GUI
//!   (`org.freedesktop.Application`) to activate for Super+V / `Toggle`.

use panora_core::error::{Error, Result};
use panora_core::model::{ClipboardData, MimePayload, Selection};
use std::collections::HashMap;
use std::sync::Mutex;
use tokio::sync::mpsc;
use tracing::debug;

/// GNOME bridge bus name.
pub const BUS_NAME: &str = "io.panora.GnomeBridge1";
/// GNOME bridge object path.
pub const OBJECT_PATH: &str = "/io/panora/GnomeBridge1";
/// Bus name owned by the Shell extension's helper service.
pub const SHELL_BUS_NAME: &str = "io.panora.GnomeShell1";
/// Object path of the Shell extension's helper service.
pub const SHELL_OBJECT_PATH: &str = "/io/panora/GnomeShell1";
/// GApplication id of the popup.
pub const GUI_APP_ID: &str = "io.panora.Panora";
/// D-Bus object path GApplication derives from the id.
pub const GUI_OBJECT_PATH: &str = "/io/panora/Panora";

/// How long one call into the Shell may take.
const CALL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

/// Small D-Bus endpoint used by the GNOME Shell extension.
pub struct GnomeBridge {
    sender: Mutex<mpsc::Sender<ClipboardData>>,
    needs_bridge: bool,
}

impl GnomeBridge {
    /// Create a bridge and bounded event receiver.
    pub fn new(capacity: usize, needs_bridge: bool) -> (Self, mpsc::Receiver<ClipboardData>) {
        let (sender, receiver) = mpsc::channel(capacity);
        (
            Self {
                sender: Mutex::new(sender),
                needs_bridge,
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

    /// Protocol version, so the extension can detect an incompatible daemon.
    #[zbus(property)]
    fn version(&self) -> u32 {
        2
    }

    /// Whether capture depends on the extension forwarding clipboard
    /// changes. False once the compositor offers a data-control protocol,
    /// so the extension can skip reading the clipboard altogether.
    #[zbus(property)]
    fn needs_bridge(&self) -> bool {
        self.needs_bridge
    }
}

#[zbus::proxy(
    interface = "io.panora.GnomeShell1",
    default_service = "io.panora.GnomeShell1",
    default_path = "/io/panora/GnomeShell1"
)]
trait Shell {
    /// Put one payload on the clipboard through St.Clipboard.
    fn set_clipboard(&self, mime: &str, bytes: &[u8]) -> zbus::Result<()>;
    /// Send Ctrl+V to the focused window through a Clutter virtual keyboard.
    fn paste(&self) -> zbus::Result<()>;
}

#[zbus::proxy(
    interface = "org.freedesktop.Application",
    default_service = "io.panora.Panora",
    default_path = "/io/panora/Panora"
)]
trait FreedesktopApplication {
    fn activate(&self, platform_data: HashMap<&str, zbus::zvariant::Value<'_>>)
        -> zbus::Result<()>;
}

fn dbus_err(context: &str, e: impl std::fmt::Display) -> Error {
    Error::Backend(format!("{context}: {e}"))
}

async fn session() -> Result<zbus::Connection> {
    zbus::Connection::session()
        .await
        .map_err(|e| dbus_err("session bus unavailable", e))
}

async fn shell_proxy(connection: &zbus::Connection) -> Result<ShellProxy<'_>> {
    ShellProxy::builder(connection)
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
        .await
        .map_err(|e| dbus_err("GNOME Shell helper unavailable", e))
}

/// Ask the Shell extension to set the clipboard content.
pub async fn shell_set_clipboard(mime: &str, bytes: &[u8]) -> Result<()> {
    let connection = session().await?;
    let proxy = shell_proxy(&connection).await?;
    tokio::time::timeout(CALL_TIMEOUT, proxy.set_clipboard(mime, bytes))
        .await
        .map_err(|_| Error::Backend("GNOME Shell helper timed out".into()))?
        .map_err(|e| dbus_err("GNOME Shell SetClipboard failed", e))
}

/// Ask the Shell extension to synthesize Ctrl+V.
pub async fn shell_paste() -> Result<()> {
    let connection = session().await?;
    let proxy = shell_proxy(&connection).await?;
    tokio::time::timeout(CALL_TIMEOUT, proxy.paste())
        .await
        .map_err(|_| Error::Backend("GNOME Shell helper timed out".into()))?
        .map_err(|e| dbus_err("GNOME Shell Paste failed", e))
}

/// Show or hide the popup. GApplication toggles on a second activation, and
/// D-Bus activation starts it outside panod's systemd sandbox when it is not
/// running. Falls back to spawning the binary for source checkouts without
/// the service file installed.
pub async fn activate_gui() -> Result<()> {
    let via_bus = async {
        let connection = session().await?;
        let proxy = FreedesktopApplicationProxy::new(&connection)
            .await
            .map_err(|e| dbus_err("GUI activation unavailable", e))?;
        tokio::time::timeout(CALL_TIMEOUT, proxy.activate(HashMap::new()))
            .await
            .map_err(|_| Error::Backend("GUI activation timed out".into()))?
            .map_err(|e| dbus_err("GUI activation failed", e))
    };
    match via_bus.await {
        Ok(()) => Ok(()),
        Err(e) => {
            debug!(error = %e, "D-Bus activation failed; spawning panora-gui");
            spawn_gui()
        }
    }
}

/// Start the popup without D-Bus activation. `systemd-run --user` is tried
/// first so the GUI does not inherit panod's sandbox (read-only home,
/// W^X memory) when panod runs as a systemd service; a plain spawn covers
/// source checkouts started from a terminal.
fn spawn_gui() -> Result<()> {
    let sibling = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("panora-gui")))
        .filter(|p| p.exists());
    let program = sibling.unwrap_or_else(|| std::path::PathBuf::from("panora-gui"));
    let quiet = |command: &mut std::process::Command| {
        command
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
    };
    if std::env::var_os("INVOCATION_ID").is_some() {
        let mut command = std::process::Command::new("systemd-run");
        command
            .args(["--user", "--quiet", "--collect", "--"])
            .arg(&program);
        quiet(&mut command);
        if let Ok(status) = command.status() {
            if status.success() {
                return Ok(());
            }
        }
    }
    let mut command = std::process::Command::new(&program);
    quiet(&mut command);
    command
        .spawn()
        .map(|_| ())
        .map_err(|e| Error::Backend(format!("cannot start panora-gui: {e}")))
}
