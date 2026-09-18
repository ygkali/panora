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
/// Well-known name GNOME Shell owns on the session bus. The extension runs
/// inside gnome-shell and pushes over that process's shared session
/// connection, so a legitimate push always arrives from this name's owner.
pub const SHELL_WELL_KNOWN_NAME: &str = "org.gnome.Shell";

/// How long one call into the Shell may take.
const CALL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);
/// A cold GTK start on a slow disk can take a while; the D-Bus default is 25 s.
const ACTIVATE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(25);

/// Small D-Bus endpoint used by the GNOME Shell extension.
pub struct GnomeBridge {
    sender: Mutex<mpsc::Sender<ClipboardData>>,
    needs_bridge: bool,
    /// Unique bus name of the GNOME Shell connection, from the last
    /// successful owner lookup. See `ensure_from_shell`.
    shell_sender: Mutex<Option<String>>,
}

impl GnomeBridge {
    /// Create a bridge and bounded event receiver.
    pub fn new(capacity: usize, needs_bridge: bool) -> (Self, mpsc::Receiver<ClipboardData>) {
        let (sender, receiver) = mpsc::channel(capacity);
        (
            Self {
                sender: Mutex::new(sender),
                needs_bridge,
                shell_sender: Mutex::new(None),
            },
            receiver,
        )
    }
}

impl GnomeBridge {
    /// Whether `sender` is the GNOME Shell connection seen last.
    fn sender_is_cached_shell(&self, sender: &str) -> bool {
        self.shell_sender
            .lock()
            .ok()
            .and_then(|slot| slot.clone())
            .is_some_and(|cached| cached == sender)
    }

    /// Remember the GNOME Shell connection so the next push needs no lookup.
    fn remember_shell_sender(&self, sender: &str) {
        if let Ok(mut slot) = self.shell_sender.lock() {
            *slot = Some(sender.to_string());
        }
    }

    /// Reject clipboard pushes that did not come from GNOME Shell.
    ///
    /// The bridge is exported on the session bus, which every process in the
    /// user's session can reach -- including a sandboxed application holding
    /// only `--socket=session-bus`, which has no access to Panora's data
    /// directory or its 0600 IPC socket. Without this check any of them could
    /// fabricate history entries, and by choosing `offered_mimes` also decide
    /// which privacy gate those entries are judged by. A malicious *Shell
    /// extension* is already outside the threat model (ADR 0003), so tying
    /// pushes to gnome-shell's own connection is the boundary that matters.
    ///
    /// Fail-closed is cheap here: the extension pushes again on the next copy.
    async fn ensure_from_shell(
        &self,
        connection: &zbus::Connection,
        header: &zbus::message::Header<'_>,
    ) -> zbus::fdo::Result<()> {
        let denied = || {
            zbus::fdo::Error::AccessDenied(
                "only GNOME Shell may push clipboard content to Panora".into(),
            )
        };
        let Some(sender) = header.sender() else {
            return Err(denied());
        };
        if self.sender_is_cached_shell(sender.as_str()) {
            return Ok(());
        }
        let name = zbus::names::BusName::try_from(SHELL_WELL_KNOWN_NAME)
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        // Property caching would add a GetAll round trip per proxy; this one
        // is built only on a cache miss and needs a single method call.
        let owner = zbus::fdo::DBusProxy::builder(connection)
            .cache_properties(zbus::proxy::CacheProperties::No)
            .build()
            .await?
            .get_name_owner(name)
            .await?;
        if owner.as_str() != sender.as_str() {
            debug!(%sender, %owner, "rejected a clipboard push from a foreign bus peer");
            return Err(denied());
        }
        self.remember_shell_sender(sender.as_str());
        Ok(())
    }

    async fn forward(&self, data: ClipboardData) -> zbus::fdo::Result<()> {
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

/// Upper bound on formats per `PushMany` call (text, html, uri-list, …).
const MAX_BRIDGE_PAYLOADS: usize = 8;

#[zbus::interface(name = "io.panora.GnomeBridge1")]
impl GnomeBridge {
    /// Receive a clipboard payload from GNOME Shell.
    async fn push(
        &self,
        mimes: Vec<String>,
        mime: String,
        bytes: Vec<u8>,
        source_app: String,
        #[zbus(connection)] connection: &zbus::Connection,
        #[zbus(header)] message: zbus::message::Header<'_>,
    ) -> zbus::fdo::Result<()> {
        self.ensure_from_shell(connection, &message).await?;
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
        self.forward(data).await
    }

    /// Receive every format of one clipboard change at once (text + HTML,
    /// or uri-list + text), so bridge entries carry the same fidelity as
    /// native captures and hash to one entry.
    async fn push_many(
        &self,
        mimes: Vec<String>,
        payloads: Vec<(String, Vec<u8>)>,
        source_app: String,
        #[zbus(connection)] connection: &zbus::Connection,
        #[zbus(header)] message: zbus::message::Header<'_>,
    ) -> zbus::fdo::Result<()> {
        self.ensure_from_shell(connection, &message).await?;
        if payloads.is_empty() || payloads.len() > MAX_BRIDGE_PAYLOADS {
            return Err(zbus::fdo::Error::InvalidArgs(format!(
                "between 1 and {MAX_BRIDGE_PAYLOADS} payloads expected"
            )));
        }
        let payloads: Vec<MimePayload> = payloads
            .into_iter()
            .map(|(mime, data)| MimePayload { mime, data })
            .collect();
        let data = ClipboardData {
            selection: Selection::Clipboard,
            offered_mimes: if mimes.is_empty() {
                payloads.iter().map(|p| p.mime.clone()).collect()
            } else {
                mimes
            },
            payloads,
            source_app: if source_app.trim().is_empty() {
                None
            } else {
                Some(source_app)
            },
        };
        self.forward(data).await
    }

    /// Protocol version, so the extension can detect an incompatible daemon.
    #[zbus(property)]
    fn version(&self) -> u32 {
        3
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
/// running. Falls back to spawning the binary only when no D-Bus service
/// for the popup exists (source checkouts); a slow cold start must not be
/// mistaken for that, or the fallback would open a second instance that
/// immediately toggles the first one closed.
pub async fn activate_gui() -> Result<()> {
    let connection = session().await?;
    let proxy = FreedesktopApplicationProxy::new(&connection)
        .await
        .map_err(|e| dbus_err("GUI activation unavailable", e))?;
    match tokio::time::timeout(ACTIVATE_TIMEOUT, proxy.activate(HashMap::new())).await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) if is_service_unknown(&e) => {
            debug!(error = %e, "no D-Bus service for the popup; spawning panora-gui");
            spawn_gui().await
        }
        Ok(Err(e)) => Err(dbus_err("GUI activation failed", e)),
        Err(_) => Err(Error::Backend("GUI activation timed out".into())),
    }
}

/// The bus knows no owner and no activatable service for the name.
fn is_service_unknown(e: &zbus::Error) -> bool {
    match e {
        zbus::Error::MethodError(name, ..) => matches!(
            name.as_str(),
            "org.freedesktop.DBus.Error.ServiceUnknown"
                | "org.freedesktop.DBus.Error.NameHasNoOwner"
        ),
        zbus::Error::FDO(fdo) => matches!(
            **fdo,
            zbus::fdo::Error::ServiceUnknown(_) | zbus::fdo::Error::NameHasNoOwner(_)
        ),
        _ => false,
    }
}

/// Start the popup without D-Bus activation. `systemd-run --user` is tried
/// first so the GUI does not inherit panod's sandbox (read-only home,
/// W^X memory) when panod runs as a systemd service; a plain spawn covers
/// source checkouts started from a terminal.
async fn spawn_gui() -> Result<()> {
    let sibling = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("panora-gui")))
        .filter(|p| p.exists());
    let program = sibling.unwrap_or_else(|| std::path::PathBuf::from("panora-gui"));
    if std::env::var_os("INVOCATION_ID").is_some() {
        let status = tokio::process::Command::new("systemd-run")
            .args(["--user", "--quiet", "--collect", "--"])
            .arg(&program)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await;
        if matches!(status, Ok(s) if s.success()) {
            return Ok(());
        }
    }
    tokio::process::Command::new(&program)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| Error::Backend(format!("cannot start panora-gui: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bridge() -> GnomeBridge {
        GnomeBridge::new(1, true).0
    }

    #[test]
    fn shell_sender_cache_starts_empty() {
        let bridge = bridge();
        assert!(!bridge.sender_is_cached_shell(":1.42"));
    }

    #[test]
    fn shell_sender_cache_matches_only_the_remembered_peer() {
        let bridge = bridge();
        bridge.remember_shell_sender(":1.42");
        assert!(bridge.sender_is_cached_shell(":1.42"));
        // A different peer on the same bus must still be looked up (and, not
        // owning org.gnome.Shell, rejected) instead of riding the cache.
        assert!(!bridge.sender_is_cached_shell(":1.43"));
        assert!(!bridge.sender_is_cached_shell(""));
    }

    #[test]
    fn shell_sender_cache_follows_a_restarted_shell() {
        let bridge = bridge();
        bridge.remember_shell_sender(":1.42");
        bridge.remember_shell_sender(":1.77");
        assert!(!bridge.sender_is_cached_shell(":1.42"));
        assert!(bridge.sender_is_cached_shell(":1.77"));
    }
}
