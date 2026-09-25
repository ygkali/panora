// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! Full-content view for one entry: complete text, image, format list.

use crate::util::{call, format_size, kind_label, qr_picture, spawn, unix_now};
use crate::window::{color_swatch, decode_scaled, recall, recall_as, toast, DecodedImage, Ui};
use gtk::gdk;
use gtk4 as gtk;
use libadwaita as adw;
use libadwaita::prelude::*;
use panora_core::i18n::fill;
use panora_core::ipc::{Request, ResponseData};
use panora_core::model::{ContentKind, Entry, MimePayload, TEXT_MIMES};
use std::rc::Rc;

/// Open the details dialog for `entry`. The payload can be megabytes and an
/// image needs a decode; both happen on a worker thread and the dialog
/// opens when they are ready.
pub fn show(ui: &Rc<Ui>, entry: &Entry) {
    let ui = ui.clone();
    let entry = entry.clone();
    let id = entry.id;
    let wants_image = matches!(entry.kind, ContentKind::Image);
    spawn(
        move || {
            let payloads = match call(&Request::Preview {
                id,
                thumbnail: false,
            }) {
                Ok(ResponseData::Payloads(payloads)) => payloads,
                _ => Vec::new(),
            };
            let image = if wants_image {
                payloads
                    .iter()
                    .find(|p| p.mime.starts_with("image/"))
                    .and_then(|p| decode_scaled(&p.data, 1024, 1024))
            } else {
                None
            };
            (payloads, image)
        },
        move |(payloads, image)| present(&ui, &entry, payloads, image),
    );
}

/// Build and present the dialog once the content is in hand.
fn present(ui: &Rc<Ui>, entry: &Entry, payloads: Vec<MimePayload>, image: Option<DecodedImage>) {
    let s = ui.s;
    let text = plain_text(&payloads);
    // UI-22: a limited HTML preview, only for entries that actually carry
    // HTML (a `RichText` entry can instead be RTF-only, which this does
    // not parse -- it falls back to the plain-text payload like before).
    let html = (entry.kind == ContentKind::RichText)
        .then(|| html_payload(&payloads))
        .flatten();

    let dialog = adw::Dialog::builder()
        .title(s.details_title)
        .content_width(560)
        .content_height(520)
        .build();

    let header = adw::HeaderBar::new();
    let mut subtitle = format!(
        "{} · {}",
        entry.primary_mime,
        format_size(s, entry.size_bytes)
    );
    if let Some(image) = &image {
        let (width, height) = image.size();
        if width > 0 && height > 0 {
            subtitle.push_str(&format!(" · {width} × {height} px"));
        }
    }
    let title = adw::WindowTitle::new(kind_label(s, entry.kind), &subtitle);
    header.set_title_widget(Some(&title));

    let body = gtk::Box::new(gtk::Orientation::Vertical, 12);
    body.set_margin_top(12);
    body.set_margin_bottom(12);
    body.set_margin_start(16);
    body.set_margin_end(16);

    if entry.sensitive {
        let note = gtk::Label::new(Some(s.details_sensitive_note));
        note.add_css_class("caption");
        note.add_css_class("warning");
        note.set_wrap(true);
        note.set_xalign(0.0);
        body.append(&note);
    }

    match entry.kind {
        ContentKind::Image => {
            let picture = gtk::Picture::new();
            picture.set_can_shrink(true);
            picture.set_content_fit(gtk::ContentFit::Contain);
            picture.set_vexpand(true);
            picture.add_css_class("details-image");
            if let Some(image) = &image {
                picture.set_paintable(Some(&image.texture()));
            }
            body.append(&picture);
        }
        ContentKind::Color => {
            body.append(&color_swatch(&entry.preview));
            body.append(&text_view(text.as_deref().unwrap_or(&entry.preview), false));
        }
        _ => {
            if let Some(html) = &html {
                body.append(&rich_text_view(html));
            } else {
                // UI-10: monospace in the full-text view too, for the same
                // reason as the card preview -- only plain `Text`, since
                // rich text and links have their own rendering.
                let is_text = entry.kind == ContentKind::Text;
                if let Some(text) = &text {
                    let code = is_text && panora_core::code::looks_like_code(text);
                    body.append(&text_view(text, code));
                } else if !entry.preview.starts_with('[') {
                    let code = is_text && panora_core::code::looks_like_code(&entry.preview);
                    body.append(&text_view(&entry.preview, code));
                } else {
                    let label = gtk::Label::new(Some(s.details_no_text));
                    label.add_css_class("dim-label");
                    label.set_vexpand(true);
                    body.append(&label);
                }
            }
        }
    }

    if !payloads.is_empty() {
        let formats = payloads
            .iter()
            .map(|p| format!("{} ({})", p.mime, format_size(s, p.data.len() as i64)))
            .collect::<Vec<_>>()
            .join("  ·  ");
        let label = gtk::Label::new(Some(&format!("{}: {formats}", s.details_formats)));
        label.add_css_class("card-meta");
        label.set_wrap(true);
        label.set_xalign(0.0);
        body.append(&label);
    }

    // What the content itself affords: a link opens or becomes a QR code,
    // a colour converts, a file list opens its folder, an image is saved.
    let extras = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    extras.set_halign(gtk::Align::Start);
    extras.set_hexpand(true);
    match entry.kind {
        ContentKind::Link => {
            if let Some(url) = text.as_deref().map(str::trim).filter(|t| looks_like_url(t)) {
                let open = gtk::Button::with_label(s.details_open_link);
                {
                    let ui = ui.clone();
                    let url = url.to_string();
                    open.connect_clicked(move |_| open_uri(&ui, &url));
                }
                extras.append(&open);
                let qr = gtk::Button::with_label(s.details_show_qr);
                {
                    let ui = ui.clone();
                    let url = url.to_string();
                    qr.connect_clicked(move |_| show_qr(&ui, &url));
                }
                extras.append(&qr);
            }
        }
        ContentKind::Color => {
            if let Ok(rgba) = gdk::RGBA::parse(entry.preview.trim()) {
                let menu = gtk::MenuButton::builder().label(s.details_copy_as).build();
                let popover = gtk::Popover::new();
                let list = gtk::Box::new(gtk::Orientation::Vertical, 2);
                for value in color_formats(&rgba) {
                    let button = gtk::Button::with_label(&value);
                    button.add_css_class("flat");
                    let ui = ui.clone();
                    let popover = popover.clone();
                    let dialog = dialog.clone();
                    button.connect_clicked(move |_| {
                        popover.popdown();
                        if store_text(&ui, &value) {
                            toast(&ui, ui.s.toast_copied_text);
                            dialog.close();
                        }
                    });
                    list.append(&button);
                }
                popover.set_child(Some(&list));
                menu.set_popover(Some(&popover));
                extras.append(&menu);
            }
        }
        ContentKind::FileList => {
            if let Some(uri) = first_file_uri(&payloads) {
                let open = gtk::Button::with_label(s.details_open_folder);
                let ui = ui.clone();
                open.connect_clicked(move |_| open_containing_folder(&ui, &uri));
                extras.append(&open);
            }
        }
        ContentKind::Image => {
            if let Some(payload) = payloads.iter().find(|p| p.mime.starts_with("image/")) {
                let save = gtk::Button::with_label(s.details_save_image);
                let ui = ui.clone();
                let bytes = payload.data.clone();
                let mime = payload.mime.clone();
                save.connect_clicked(move |_| save_image(&ui, bytes.clone(), &mime));
                extras.append(&save);
            }
        }
        _ => {}
    }

    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    actions.set_halign(gtk::Align::End);

    if text.is_some() && payloads.len() > 1 {
        let plain = gtk::Button::with_label(s.details_copy_plain);
        let ui = ui.clone();
        let dialog = dialog.clone();
        let id = entry.id;
        plain.connect_clicked(move |_| {
            if recall_as(&ui, id, "text/plain") {
                toast(&ui, ui.s.toast_copied_text);
                dialog.close();
            }
        });
        actions.append(&plain);
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
    let footer = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    footer.append(&extras);
    footer.append(&actions);
    body.append(&footer);

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

/// The HTML payload of an entry, if any (`text/html`, with or without a
/// `;charset=...` parameter).
fn html_payload(payloads: &[MimePayload]) -> Option<String> {
    payloads
        .iter()
        .find(|p| p.mime.starts_with("text/html"))
        .and_then(|p| String::from_utf8(p.data.clone()).ok())
}

/// UI-22: `html` rendered as a limited Pango-markup preview -- bold,
/// italic, underline, strikethrough, inline code, links (shown styled, not
/// clickable) and paragraph/line breaks. See
/// `panora_core::richtext::html_to_pango` for exactly what is and is not
/// carried over; this is a preview, not a renderer.
fn rich_text_view(html: &str) -> gtk::ScrolledWindow {
    let view = gtk::TextView::new();
    view.set_editable(false);
    view.set_cursor_visible(false);
    view.set_wrap_mode(gtk::WrapMode::WordChar);
    view.set_left_margin(8);
    view.set_right_margin(8);
    view.set_top_margin(8);
    view.set_bottom_margin(8);
    let buffer = view.buffer();
    let mut end = buffer.end_iter();
    buffer.insert_markup(&mut end, &panora_core::richtext::html_to_pango(html));
    gtk::ScrolledWindow::builder()
        .vexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&view)
        .build()
}

/// `code` picks a monospace font (UI-10): the caller already knows the
/// entry's kind, so the heuristic runs once there rather than per view.
fn text_view(text: &str, code: bool) -> gtk::ScrolledWindow {
    let view = gtk::TextView::new();
    view.set_editable(false);
    view.set_cursor_visible(false);
    view.set_wrap_mode(gtk::WrapMode::WordChar);
    view.set_left_margin(8);
    view.set_right_margin(8);
    view.set_top_margin(8);
    view.set_bottom_margin(8);
    if code {
        view.add_css_class("code-preview");
    }
    view.buffer().set_text(text);
    gtk::ScrolledWindow::builder()
        .vexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&view)
        .build()
}

fn looks_like_url(text: &str) -> bool {
    (text.starts_with("http://") || text.starts_with("https://") || text.starts_with("mailto:"))
        && !text.contains(char::is_whitespace)
}

/// Hand a URL to the default browser (or mail client).
fn open_uri(ui: &Rc<Ui>, uri: &str) {
    let window = ui.window.clone();
    let ui = ui.clone();
    gtk::UriLauncher::new(uri).launch(
        Some(&window),
        None::<&gtk::gio::Cancellable>,
        move |result| {
            if let Err(e) = result {
                toast(&ui, &fill(ui.s.toast_open_failed, "e", &e.to_string()));
            }
        },
    );
}

/// Show the URL as a QR code so a phone can open it: no network, no
/// account, just the camera.
fn show_qr(ui: &Rc<Ui>, url: &str) {
    let Some(picture) = qr_picture(url, 6) else {
        toast(
            ui,
            &fill(ui.s.toast_open_failed, "e", "too long for a QR code"),
        );
        return;
    };
    picture.set_margin_top(12);
    let caption = gtk::Label::new(Some(url));
    caption.add_css_class("dim-label");
    caption.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    caption.set_max_width_chars(40);
    caption.set_margin_top(8);
    caption.set_margin_bottom(16);
    caption.set_margin_start(16);
    caption.set_margin_end(16);
    let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
    column.append(&picture);
    column.append(&caption);
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());
    toolbar.set_content(Some(&column));
    let dialog = adw::Dialog::builder().title(ui.s.details_qr_title).build();
    dialog.set_child(Some(&toolbar));
    dialog.present(Some(&ui.window));
}

/// The same colour as hex, `rgb()` and `hsl()`.
fn color_formats(rgba: &gdk::RGBA) -> Vec<String> {
    let channel = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    let (r, g, b) = (
        channel(rgba.red()),
        channel(rgba.green()),
        channel(rgba.blue()),
    );
    let (h, s, l) = rgb_to_hsl(rgba.red(), rgba.green(), rgba.blue());
    vec![
        format!("#{r:02x}{g:02x}{b:02x}"),
        format!("rgb({r}, {g}, {b})"),
        format!(
            "hsl({}, {}%, {}%)",
            h.round() as i32,
            (s * 100.0).round() as i32,
            (l * 100.0).round() as i32
        ),
    ]
}

fn rgb_to_hsl(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    if (max - min).abs() < f32::EPSILON {
        return (0.0, 0.0, l);
    }
    let d = max - min;
    let s = if l > 0.5 {
        d / (2.0 - max - min)
    } else {
        d / (max + min)
    };
    let h = if (max - r).abs() < f32::EPSILON {
        (g - b) / d + if g < b { 6.0 } else { 0.0 }
    } else if (max - g).abs() < f32::EPSILON {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (h * 60.0, s, l)
}

/// Record `text` as a new entry and put it on the clipboard.
fn store_text(ui: &Rc<Ui>, text: &str) -> bool {
    let request = Request::Store {
        payloads: vec![MimePayload::new(
            "text/plain;charset=utf-8",
            text.as_bytes(),
        )],
        source_app: None,
        copy: true,
    };
    match call(&request) {
        Ok(_) => true,
        Err(_) => {
            toast(ui, ui.s.toast_recall_failed);
            false
        }
    }
}

/// The first local file of a copied file list.
fn first_file_uri(payloads: &[MimePayload]) -> Option<String> {
    let list = payloads.iter().find(|p| p.mime == "text/uri-list")?;
    String::from_utf8_lossy(&list.data)
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with("file://"))
        .map(String::from)
}

/// Reveal the file in the file manager.
fn open_containing_folder(ui: &Rc<Ui>, uri: &str) {
    let file = gtk::gio::File::for_uri(uri);
    let window = ui.window.clone();
    let ui = ui.clone();
    gtk::FileLauncher::new(Some(&file)).open_containing_folder(
        Some(&window),
        None::<&gtk::gio::Cancellable>,
        move |result| {
            if let Err(e) = result {
                toast(&ui, &fill(ui.s.toast_open_failed, "e", &e.to_string()));
            }
        },
    );
}

/// Write the image payload to a file the user picks.
fn save_image(ui: &Rc<Ui>, bytes: Vec<u8>, mime: &str) {
    let extension = match mime {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/webp" => "webp",
        "image/gif" => "gif",
        "image/bmp" => "bmp",
        "image/tiff" => "tiff",
        _ => "img",
    };
    let dialog = gtk::FileDialog::builder()
        .title(ui.s.details_save_image)
        .initial_name(format!("clipboard-{}.{extension}", unix_now()))
        .build();
    let window = ui.window.clone();
    let ui = ui.clone();
    dialog.save(
        Some(&window),
        None::<&gtk::gio::Cancellable>,
        move |result| match result {
            Ok(file) => match file.path() {
                Some(path) => match std::fs::write(&path, &bytes) {
                    Ok(()) => toast(
                        &ui,
                        &fill(ui.s.toast_saved, "p", &path.display().to_string()),
                    ),
                    Err(e) => toast(&ui, &fill(ui.s.toast_save_failed, "e", &e.to_string())),
                },
                None => toast(&ui, &fill(ui.s.toast_save_failed, "e", "not a local file")),
            },
            Err(e) if e.matches(gtk::DialogError::Dismissed) => {}
            Err(e) => toast(&ui, &fill(ui.s.toast_save_failed, "e", &e.to_string())),
        },
    );
}
