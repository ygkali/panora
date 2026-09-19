// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! First-run welcome: three short pages on the shortcut, the privacy model
//! and what this session can do. Shown once; a `first-run` file next to
//! `config.toml` records that it was seen.

use crate::util::call_async;
use crate::window::Ui;
use gtk4 as gtk;
use libadwaita as adw;
use libadwaita::prelude::*;
use panora_core::config::config_path;
use panora_core::i18n::Strings;
use panora_core::ipc::{CapabilityData, Request, ResponseData};
use std::path::PathBuf;
use std::rc::Rc;

/// The file whose presence means the welcome was shown.
pub fn marker_path() -> PathBuf {
    config_path().with_file_name("first-run")
}

/// Show the welcome unless it was shown before. The session page depends
/// on what the daemon reports, so the dialog waits for `Status`.
pub fn show_if_first_run(ui: &Rc<Ui>) {
    if marker_path().exists() {
        return;
    }
    let ui = ui.clone();
    call_async(Request::Status, move |result| {
        let capabilities = match result {
            Ok(ResponseData::Status(status)) => Some(status.capabilities),
            _ => None,
        };
        present(&ui, capabilities.as_ref());
    });
}

fn present(ui: &Rc<Ui>, capabilities: Option<&CapabilityData>) {
    let s = ui.s;
    let dialog = adw::Dialog::builder()
        .title(s.welcome_title)
        .content_width(400)
        .content_height(520)
        .build();

    let carousel = adw::Carousel::new();
    carousel.set_vexpand(true);
    carousel.set_hexpand(true);
    carousel.append(&page(
        "input-keyboard-symbolic",
        s.welcome_shortcut_title,
        s.welcome_shortcut_body,
    ));
    carousel.append(&page(
        "security-high-symbolic",
        s.welcome_privacy_title,
        s.welcome_privacy_body,
    ));
    carousel.append(&page(
        "computer-symbolic",
        s.welcome_session_title,
        &session_notes(s, capabilities),
    ));

    let dots = adw::CarouselIndicatorDots::new();
    dots.set_carousel(Some(&carousel));

    let next = gtk::Button::with_label(s.welcome_next);
    next.add_css_class("pill");
    next.add_css_class("suggested-action");
    next.set_halign(gtk::Align::Center);
    next.set_margin_top(6);
    next.set_margin_bottom(18);
    {
        let carousel = carousel.clone();
        let dialog = dialog.clone();
        next.connect_clicked(move |_| {
            let position = carousel.position().round() as u32;
            if position + 1 >= carousel.n_pages() {
                dialog.close();
            } else {
                carousel.scroll_to(&carousel.nth_page(position + 1), true);
            }
        });
    }
    {
        let next = next.clone();
        let (next_label, done_label) = (s.welcome_next, s.welcome_done);
        carousel.connect_page_changed(move |carousel, index| {
            next.set_label(if index + 1 >= carousel.n_pages() {
                done_label
            } else {
                next_label
            });
        });
    }

    let body = gtk::Box::new(gtk::Orientation::Vertical, 6);
    body.append(&carousel);
    body.append(&dots);
    body.append(&next);

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());
    toolbar.set_content(Some(&body));
    dialog.set_child(Some(&toolbar));
    // Closing by any route counts as seen; the pages are all in the README.
    dialog.connect_closed(|_| mark_seen());
    dialog.present(Some(&ui.window));
}

fn page(icon: &str, title: &str, body: &str) -> adw::StatusPage {
    adw::StatusPage::builder()
        .icon_name(icon)
        .title(title)
        // The description is Pango markup; the texts are this crate's own,
        // but an ampersand in a translation must not break the page.
        .description(glib::markup_escape_text(body).as_str())
        .build()
}

/// What this session cannot do, from the daemon's capabilities; one line
/// per limitation, or a reassurance when there is none.
fn session_notes(s: &Strings, capabilities: Option<&CapabilityData>) -> String {
    let Some(caps) = capabilities else {
        return s.welcome_session_unknown.to_string();
    };
    let mut notes = Vec::new();
    if caps.needs_bridge {
        notes.push(s.welcome_session_bridge);
    }
    if !caps.source_app {
        notes.push(s.welcome_session_no_source_app);
    }
    if !caps.synthetic_paste {
        notes.push(s.welcome_session_no_paste);
    }
    if notes.is_empty() {
        s.welcome_session_all_good.to_string()
    } else {
        notes.join("\n\n")
    }
}

fn mark_seen() {
    let path = marker_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(&path, b"seen\n");
}
