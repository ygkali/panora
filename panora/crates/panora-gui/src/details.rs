// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! Full-content view for one entry: complete text, image, format list.

use crate::util::{call, format_size, kind_label};
use crate::window::{color_swatch, recall, texture_from_bytes, toast, Ui};
use gtk4 as gtk;
use libadwaita as adw;
use libadwaita::prelude::*;
use panora_core::ipc::{Request, ResponseData};
use panora_core::model::{ContentKind, Entry, MimePayload, TEXT_MIMES};
use std::rc::Rc;

/// Open the details dialog for `entry`.
pub fn show(ui: &Rc<Ui>, entry: &Entry) {
    let s = ui.s;
    let payloads = match call(&Request::Preview { id: entry.id }) {
        Ok(ResponseData::Payloads(payloads)) => payloads,
        _ => Vec::new(),
    };
    let text = plain_text(&payloads);

    let dialog = adw::Dialog::builder()
        .title(s.details_title)
        .content_width(560)
        .content_height(520)
        .build();

    let header = adw::HeaderBar::new();
    let title = adw::WindowTitle::new(
        kind_label(s, entry.kind),
        &format!("{} · {}", entry.primary_mime, format_size(entry.size_bytes)),
    );
    header.set_title_widget(Some(&title));

    let body = gtk::Box::new(gtk::Orientation::Vertical, 12);
    body.set_margin_top(12);
    body.set_margin_bottom(12);
    body.set_margin_start(16);
    body.set_margin_end(16);

    match entry.kind {
        ContentKind::Image => {
            let picture = gtk::Picture::new();
            picture.set_can_shrink(true);
            picture.set_content_fit(gtk::ContentFit::Contain);
            picture.set_vexpand(true);
            picture.add_css_class("details-image");
            if let Some(image) = payloads.iter().find(|p| p.mime.starts_with("image/")) {
                if let Some(texture) = texture_from_bytes(&image.data, 1024, 1024) {
                    picture.set_paintable(Some(&texture));
                }
            }
            body.append(&picture);
        }
        ContentKind::Color => {
            body.append(&color_swatch(&entry.preview));
            body.append(&text_view(text.as_deref().unwrap_or(&entry.preview)));
        }
        _ => {
            if let Some(text) = &text {
                body.append(&text_view(text));
            } else if !entry.preview.starts_with('[') {
                body.append(&text_view(&entry.preview));
            } else {
                let label = gtk::Label::new(Some(s.details_no_text));
                label.add_css_class("dim-label");
                label.set_vexpand(true);
                body.append(&label);
            }
        }
    }

    if !payloads.is_empty() {
        let formats = payloads
            .iter()
            .map(|p| format!("{} ({})", p.mime, format_size(p.data.len() as i64)))
            .collect::<Vec<_>>()
            .join("  ·  ");
        let label = gtk::Label::new(Some(&format!("{}: {formats}", s.details_formats)));
        label.add_css_class("card-meta");
        label.set_wrap(true);
        label.set_xalign(0.0);
        body.append(&label);
    }

    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    actions.set_halign(gtk::Align::End);

    if let Some(text) = text.clone() {
        if payloads.len() > 1 {
            let plain = gtk::Button::with_label(s.details_copy_plain);
            let ui = ui.clone();
            let dialog = dialog.clone();
            plain.connect_clicked(move |_| {
                ui.window.clipboard().set_text(&text);
                toast(&ui, ui.s.toast_copied_text);
                dialog.close();
            });
            actions.append(&plain);
        }
    }

    let copy = gtk::Button::with_label(s.details_copy);
    copy.add_css_class("suggested-action");
    {
        let ui = ui.clone();
        let dialog = dialog.clone();
        let id = entry.id;
        copy.connect_clicked(move |_| {
            dialog.close();
            recall(&ui, id);
        });
    }
    actions.append(&copy);
    body.append(&actions);

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&body));
    dialog.set_child(Some(&toolbar));
    dialog.present(Some(&ui.window));
}

/// Best plain-text payload of an entry, if any.
fn plain_text(payloads: &[MimePayload]) -> Option<String> {
    TEXT_MIMES
        .iter()
        .find_map(|m| payloads.iter().find(|p| p.mime == *m))
        .or_else(|| {
            payloads
                .iter()
                .find(|p| p.is_text() && !p.mime.starts_with("text/html"))
        })
        .and_then(|p| String::from_utf8(p.data.clone()).ok())
        .map(|t| t.trim_end_matches('\0').to_string())
}

fn text_view(text: &str) -> gtk::ScrolledWindow {
    let view = gtk::TextView::new();
    view.set_editable(false);
    view.set_cursor_visible(false);
    view.set_wrap_mode(gtk::WrapMode::WordChar);
    view.set_left_margin(8);
    view.set_right_margin(8);
    view.set_top_margin(8);
    view.set_bottom_margin(8);
    view.add_css_class("details-text");
    view.buffer().set_text(text);
    gtk::ScrolledWindow::builder()
        .vexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&view)
        .build()
}
