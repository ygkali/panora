// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! Panora's Win+V-style GTK4/libadwaita popup.
//!
//! The application is unique per session: a second launch (Super+V from the
//! GNOME extension, `panora-cli toggle`, the launcher) activates the running
//! instance, which closes its window. The result is a toggle without a
//! resident process: the popup exits as soon as it is dismissed.

#![forbid(unsafe_code)]

mod details;
#[cfg(feature = "fixture")]
mod fixture;
mod settings;
mod util;
mod window;

use gtk4 as gtk;
use libadwaita as adw;
use libadwaita::prelude::*;
use panora_core::config::Config;
use std::cell::RefCell;
use std::rc::Rc;

/// GApplication id; also the D-Bus name used for activation.
pub const APP_ID: &str = "io.github.ygkali.Panora";

/// Process-wide state shared by every window builder.
pub struct App {
    pub config: RefCell<Config>,
    pub strings: &'static panora_core::i18n::Strings,
}

fn main() -> glib::ExitCode {
    // GApplication treats any argument it does not know as a file to open,
    // so the version flag is answered before GTK sees the command line.
    if std::env::args()
        .skip(1)
        .any(|arg| arg == "--version" || arg == "-V")
    {
        println!("panora-gui {}", env!("CARGO_PKG_VERSION"));
        return glib::ExitCode::SUCCESS;
    }
    // Cairo avoids a large software/GL surface allocation in minimal X11
    // desktops; advanced users can override it through GSK_RENDERER.
    if std::env::var_os("GSK_RENDERER").is_none() {
        std::env::set_var("GSK_RENDERER", "cairo");
    }
    adw::init().expect("libadwaita initialization failed");

    let config = Config::load().unwrap_or_else(|e| {
        eprintln!("panora-gui: config ignored: {e}");
        Config::default()
    });
    let language = panora_core::i18n::Language::from_config(&config.ui.language);
    let state = Rc::new(App {
        config: RefCell::new(config),
        strings: language.strings(),
    });

    let app = adw::Application::builder()
        .application_id(APP_ID)
        .flags(gtk::gio::ApplicationFlags::default())
        .build();

    {
        let state = state.clone();
        app.connect_startup(move |_| {
            util::apply_theme(&state.config.borrow().ui.theme);
            window::install_css();
        });
    }
    {
        let state = state.clone();
        app.connect_activate(move |app| {
            // Activation while the popup is open means "toggle it off".
            if let Some(existing) = app.active_window() {
                existing.close();
                return;
            }
            window::build(app, &state);
        });
    }
    app.run()
}
