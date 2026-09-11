// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! Display-server backends: native X11, native Wayland and the GNOME Shell
//! bridge, plus the session-based selection logic.

pub mod bridge;
#[cfg(unix)]
pub mod wayland;
pub mod x11;

use panora_core::backend::ClipboardBackend;
use panora_core::error::{Error, Result};
use std::sync::Arc;
use tracing::{info, warn};

/// Pick the backend that fits the running session.
///
/// Order: a Wayland display gets the native data-control backend; if the
/// compositor lacks the protocol, GNOME sessions fall back to the Shell
/// bridge and everything else to X11 through XWayland (with a warning,
/// since app names are then unknown). Plain X11 sessions use XFIXES.
pub async fn select_backend() -> Result<Arc<dyn ClipboardBackend>> {
    let session_type = std::env::var("XDG_SESSION_TYPE").unwrap_or_default();
    let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some() && session_type != "x11";
    let x11 = std::env::var_os("DISPLAY").is_some();
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    // systemd user services do not always inherit XDG_CURRENT_DESKTOP, so
    // also look for gnome-shell on the session bus.
    let is_gnome = desktop.to_ascii_lowercase().contains("gnome");

    if wayland {
        #[cfg(unix)]
        match wayland::WaylandBackend::connect() {
            Ok(backend) => return Ok(Arc::new(backend)),
            Err(e) => warn!(error = %e, "native Wayland capture unavailable"),
        }
        if is_gnome || gnome_shell_running().await {
            info!("using the GNOME Shell bridge backend; the Panora extension must be enabled");
            return Ok(Arc::new(bridge::GnomeBridgeBackend::new()));
        }
        if x11 {
            warn!("falling back to X11 capture through XWayland; source application names will be unavailable");
            return x11::X11Backend::connect().map(|b| Arc::new(b) as Arc<dyn ClipboardBackend>);
        }
        return Err(Error::Backend(
            "Wayland compositor has no data-control protocol and no X11 display".into(),
        ));
    }
    if x11 {
        return x11::X11Backend::connect().map(|b| Arc::new(b) as Arc<dyn ClipboardBackend>);
    }
    Err(Error::Backend(
        "no display: neither WAYLAND_DISPLAY nor DISPLAY is set".into(),
    ))
}

/// True when `org.gnome.Shell` owns its name on the session bus. Async on
/// purpose: the blocking zbus API spins up its own runtime and panics when
/// called from inside ours.
async fn gnome_shell_running() -> bool {
    let Ok(connection) = zbus::Connection::session().await else {
        return false;
    };
    let Ok(proxy) = zbus::fdo::DBusProxy::new(&connection).await else {
        return false;
    };
    let Ok(name) = zbus::names::BusName::try_from("org.gnome.Shell") else {
        return false;
    };
    proxy.name_has_owner(name).await.unwrap_or(false)
}
