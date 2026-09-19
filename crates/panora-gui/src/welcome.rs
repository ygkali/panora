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
use std::cell::Cell;
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

/// The pager: a stack of pages, one dot per page, one button that moves
/// on and finally closes.
struct Pager {
    stack: gtk::Stack,
    dots: Vec<gtk::Label>,
    next: gtk::Button,
    current: Cell<usize>,
    next_label: &'static str,
    done_label: &'static str,
}

impl Pager {
    fn go_to(&self, index: usize) {
        self.current.set(index);
        self.stack.set_visible_child_name(&index.to_string());
        for (i, dot) in self.dots.iter().enumerate() {
            if i == index {
                dot.remove_css_class("dim-label");
                dot.add_css_class("accent");
            } else {
                dot.add_css_class("dim-label");
                dot.remove_css_class("accent");
            }
        }
        self.next.set_label(if index + 1 >= self.dots.len() {
            self.done_label
        } else {
            self.next_label
        });
    }
}

fn present(ui: &Rc<Ui>, capabilities: Option<&CapabilityData>) {
    let s = ui.s;
    let dialog = adw::Dialog::builder()
        .title(s.welcome_title)
        .content_width(400)
        .content_height(520)
        .build();

    let pages = [
        (
            "input-keyboard-symbolic",
            s.welcome_shortcut_title,
            s.welcome_shortcut_body.to_string(),
        ),
        (
            "security-high-symbolic",
            s.welcome_privacy_title,
            s.welcome_privacy_body.to_string(),
        ),
        (
            "computer-symbolic",
            s.welcome_session_title,
            session_notes(s, capabilities),
        ),
    ];

    // A stack, not a carousel: every page gets the dialog's full width,
    // whatever the popup's width happens to be.
    let stack = gtk::Stack::new();
    stack.set_transition_type(gtk::StackTransitionType::SlideLeftRight);
    stack.set_vexpand(true);
    stack.set_hhomogeneous(true);
    stack.set_vhomogeneous(true);
    let dots_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    dots_row.set_halign(gtk::Align::Center);
    let mut dots = Vec::new();
    for (index, (icon, title, body)) in pages.iter().enumerate() {
        stack.add_named(&page(icon, title, body), Some(&index.to_string()));
        let dot = gtk::Label::new(Some("●"));
        dot.add_css_class("dim-label");
        dots_row.append(&dot);
        dots.push(dot);
    }

    let next = gtk::Button::with_label(s.welcome_next);
    next.add_css_class("pill");
    next.add_css_class("suggested-action");
    next.set_halign(gtk::Align::Center);
    next.set_margin_top(6);
    next.set_margin_bottom(18);

    let pager = Rc::new(Pager {
        stack: stack.clone(),
        dots,
        next: next.clone(),
        current: Cell::new(0),
        next_label: s.welcome_next,
        done_label: s.welcome_done,
    });
    pager.go_to(0);
    {
        let pager = pager.clone();
        let dialog = dialog.clone();
        next.connect_clicked(move |_| {
            let index = pager.current.get();
            if index + 1 >= pager.dots.len() {
                dialog.close();
            } else {
                pager.go_to(index + 1);
            }
        });
    }

    let body = gtk::Box::new(gtk::Orientation::Vertical, 6);
    body.append(&stack);
    body.append(&dots_row);
    body.append(&next);

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());
    toolbar.set_content(Some(&body));
    dialog.set_child(Some(&toolbar));
    // Closing by any route counts as seen; the pages are all in the README.
    dialog.connect_closed(|_| mark_seen());
    dialog.present(Some(&ui.window));
}

/// One page: a large symbolic icon, a title and a paragraph, centred.
fn page(icon: &str, title: &str, body: &str) -> gtk::Box {
    let column = gtk::Box::new(gtk::Orientation::Vertical, 12);
    column.set_valign(gtk::Align::Center);
    column.set_margin_top(24);
    column.set_margin_bottom(12);
    column.set_margin_start(28);
    column.set_margin_end(28);

    let image = gtk::Image::from_icon_name(icon);
    image.set_pixel_size(72);
    image.add_css_class("dim-label");
    image.set_margin_bottom(8);
    column.append(&image);

    let heading = gtk::Label::new(Some(title));
    heading.add_css_class("title-1");
    heading.set_wrap(true);
    heading.set_justify(gtk::Justification::Center);
    column.append(&heading);

    let text = gtk::Label::new(Some(body));
    text.add_css_class("body");
    text.set_wrap(true);
    text.set_justify(gtk::Justification::Center);
    text.set_max_width_chars(40);
    column.append(&text);
    column
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
