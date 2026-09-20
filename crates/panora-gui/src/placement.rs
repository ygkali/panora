// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! Where the popup appears.
//!
//! GTK4 has no window-positioning API, so each session type gets its own
//! route: on X11 the window is put next to the pointer through the X
//! server (a user-position hint before it is mapped, and the same move
//! again right after, for servers without a window manager); on wlroots
//! compositors (Sway, Hyprland, ...) it becomes a layer-shell overlay
//! anchored to a screen corner, above fullscreen windows and without a
//! decoration; on GNOME the Shell extension moves it after activation
//! (`Meta.Window.move_frame`), since Mutter offers neither route to a
//! client.
//!
//! Layer-shell support is a build-time feature (`layer-shell`): the
//! gtk4-layer-shell library has to be linked ahead of libwayland-client,
//! which rules out loading it on demand, and Ubuntu 24.04 does not ship
//! it. Builds without the feature behave as before on those compositors.

use gtk4 as gtk;
use gtk4::prelude::*;
use panora_core::config::UiConfig;
use std::time::Duration;

/// How far above the pointer the window's top edge lands on X11.
const POINTER_OFFSET_Y: i32 = 24;
/// Delay before the post-map move on X11: GTK's first configure of the
/// mapped window has gone out by then.
const SETTLE_MS: u64 = 120;

/// `PANORA_DEBUG=1` prints what the placement did or why it did nothing.
fn note(reason: &str) {
    if std::env::var_os("PANORA_DEBUG").is_some() {
        eprintln!("panora-gui: placement: {reason}");
    }
}

/// True when GDK is talking to a Wayland compositor.
#[cfg(feature = "layer-shell")]
fn on_wayland() -> bool {
    gtk::gdk::Display::default()
        .is_some_and(|display| display.type_().name() == "GdkWaylandDisplay")
}

/// True when GDK is talking to an X server.
fn on_x11() -> bool {
    gtk::gdk::Display::default().is_some_and(|display| display.type_().name() == "GdkX11Display")
}

/// Before the window is realized: make it a layer-shell overlay where the
/// build and the compositor allow it. Returns true when it did.
#[cfg(feature = "layer-shell")]
pub fn prepare(window: &impl IsA<gtk::Window>, config: &UiConfig) -> bool {
    use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell as _};

    // `is_supported` asserts a Wayland display; ask only there.
    if !on_wayland() || !gtk4_layer_shell::is_supported() {
        note("no layer-shell here");
        return false;
    }
    let window: &gtk::Window = window.upcast_ref();
    window.init_layer_shell();
    window.set_namespace(Some("panora"));
    window.set_layer(Layer::Overlay);
    window.set_keyboard_mode(KeyboardMode::OnDemand);
    let anchors: &[Edge] = match config.layer_anchor.as_str() {
        "top-left" => &[Edge::Top, Edge::Left],
        "bottom-right" => &[Edge::Bottom, Edge::Right],
        "bottom-left" => &[Edge::Bottom, Edge::Left],
        "center" => &[],
        _ => &[Edge::Top, Edge::Right],
    };
    for edge in anchors {
        window.set_anchor(*edge, true);
        window.set_margin(*edge, 16);
    }
    note("layer-shell overlay");
    true
}

/// Without the `layer-shell` feature there is nothing to prepare.
#[cfg(not(feature = "layer-shell"))]
pub fn prepare(_window: &impl IsA<gtk::Window>, _config: &UiConfig) -> bool {
    note("built without layer-shell");
    false
}

/// Before the window is mapped: on X11, realize it and put it next to the
/// pointer, with a user-position hint so the window manager keeps it
/// there; the same move is repeated shortly after the map for X servers
/// that run no window manager. Elsewhere this is a no-op.
pub fn place(window: &impl IsA<gtk::Window>, config: &UiConfig) {
    if config.position != "pointer" {
        note("position is not pointer");
        return;
    }
    if !on_x11() {
        note("not on X11; the compositor or the extension places the window");
        return;
    }
    let window: &gtk::Window = window.upcast_ref();
    // Realizing creates the X window without mapping it, so the move and
    // the hint are in place when the window manager first sees it.
    WidgetExt::realize(window);
    let Some(target) = target_near_pointer(window) else {
        note("pointer position unavailable");
        return;
    };
    move_x11(window, target, true);
    window.connect_map(move |window| {
        let window = window.clone();
        glib::timeout_add_local_once(Duration::from_millis(SETTLE_MS), move || {
            move_x11(&window, target, false);
        });
    });
}

/// The X window id of a realized GTK window on X11.
fn xid_of(window: &gtk::Window) -> Option<u32> {
    let surface = window.surface()?;
    let surface = surface.downcast::<gdk4_x11::X11Surface>().ok()?;
    u32::try_from(surface.xid()).ok()
}

/// Where the window's top-left corner should go so the pointer sits just
/// below its top edge, kept inside the screen.
fn target_near_pointer(window: &gtk::Window) -> Option<(i32, i32)> {
    use x11rb::connection::Connection as _;
    use x11rb::protocol::xproto::ConnectionExt as _;

    let (conn, screen_index) = x11rb::connect(None).ok()?;
    let screen = &conn.setup().roots[screen_index];
    let pointer = conn.query_pointer(screen.root).ok()?.reply().ok()?;
    let (width, height) = {
        let (w, h) = (window.width(), window.height());
        if w > 0 && h > 0 {
            (w, h)
        } else {
            let (dw, dh) = window.default_size();
            (dw.max(1), dh.max(1))
        }
    };
    let screen_w = i32::from(screen.width_in_pixels);
    let screen_h = i32::from(screen.height_in_pixels);
    let x = (i32::from(pointer.root_x) - width / 2).clamp(0, (screen_w - width).max(0));
    let y = (i32::from(pointer.root_y) - POINTER_OFFSET_Y).clamp(0, (screen_h - height).max(0));
    Some((x, y))
}

/// Move the X window to `target`; with `hint`, also record the position in
/// `WM_NORMAL_HINTS` as the user's choice, which window managers honour
/// when they map the window.
fn move_x11(window: &gtk::Window, (x, y): (i32, i32), hint: bool) {
    use x11rb::connection::Connection as _;
    use x11rb::properties::{WmSizeHints, WmSizeHintsSpecification};
    use x11rb::protocol::xproto::{ConfigureWindowAux, ConnectionExt as _};

    let Some(xid) = xid_of(window) else {
        note("no X window");
        return;
    };
    let Ok((conn, _)) = x11rb::connect(None) else {
        note("no X connection");
        return;
    };
    if hint {
        // Keep GTK's own size hints and add the position.
        let mut hints = WmSizeHints::get_normal_hints(&conn, xid)
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .flatten()
            .unwrap_or_default();
        hints.position = Some((WmSizeHintsSpecification::UserSpecified, x, y));
        let _ = hints.set_normal_hints(&conn, xid);
    }
    // Two requests: a plain configure, which is all there is without a
    // window manager, and the EWMH move, which managers honour for mapped
    // windows and which a plain configure may lose to their own placement.
    match conn.configure_window(xid, &ConfigureWindowAux::new().x(x).y(y)) {
        Ok(_) => note(&format!("moved 0x{xid:x} to {x},{y} (hint: {hint})")),
        Err(e) => note(&format!("configure failed: {e}")),
    }
    if !hint {
        ewmh_move(&conn, xid, x, y);
    }
    let _ = conn.flush();
}

/// `_NET_MOVERESIZE_WINDOW` to the root window: north-west gravity, x and
/// y set, source "pager" (2), which managers treat as a user request.
fn ewmh_move(conn: &x11rb::rust_connection::RustConnection, xid: u32, x: i32, y: i32) {
    use x11rb::connection::Connection as _;
    use x11rb::protocol::xproto::{ClientMessageEvent, ConnectionExt as _, EventMask};

    let Ok(atom) = conn
        .intern_atom(false, b"_NET_MOVERESIZE_WINDOW")
        .ok()
        .and_then(|cookie| cookie.reply().ok())
        .map(|reply| reply.atom)
        .ok_or(())
    else {
        return;
    };
    let root = conn.setup().roots[0].root;
    let flags: u32 = 1 | (1 << 8) | (1 << 9) | (2 << 12);
    let event = ClientMessageEvent::new(32, xid, atom, [flags, x as u32, y as u32, 0, 0]);
    let _ = conn.send_event(
        false,
        root,
        EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
        event,
    );
}
