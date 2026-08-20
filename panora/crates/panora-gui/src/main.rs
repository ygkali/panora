// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! Panora's lightweight Win+V-style GTK4/libadwaita popup.

use gdk_pixbuf::PixbufLoader;
use gtk::gdk;
use gtk4 as gtk;
use libadwaita as adw;
use libadwaita::prelude::*;
use panora_core::config::socket_path;
use panora_core::model::{ContentKind, Entry, MimePayload};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::sync::mpsc::{self, TryRecvError};
use std::time::Duration;

#[derive(Debug, Serialize)]
#[serde(tag = "method", content = "params")]
enum Request {
    List(QueryRequest),
    Recall { id: i64 },
    Pin { id: i64, pinned: bool },
    Delete { id: i64 },
    Clear,
    SetPrivate { enabled: bool },
    Preview { id: i64 },
}

#[derive(Debug, Serialize, Default)]
struct QueryRequest {
    search: Option<String>,
    kind: Option<String>,
    pinned_only: bool,
    limit: usize,
    offset: usize,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "ok", content = "data")]
enum Response {
    #[serde(rename = "true")]
    Success(ResponseData),
    #[serde(rename = "false")]
    Failure { message: String },
}

#[derive(Debug, Deserialize)]
enum ResponseData {
    Entries(Vec<Entry>),
    Payloads(Vec<MimePayload>),
    Empty,
}

fn main() {
    // Cairo avoids a large software/GL surface allocation in minimal X11
    // desktops; advanced users can override it through GSK_RENDERER.
    if std::env::var_os("GSK_RENDERER").is_none() {
        std::env::set_var("GSK_RENDERER", "cairo");
    }
    adw::init().expect("libadwaita initialization failed");
    let app = adw::Application::builder()
        .application_id("io.panora.Panora")
        .flags(gtk::gio::ApplicationFlags::NON_UNIQUE)
        .build();
    app.connect_activate(build_ui);
    app.run();
}

fn build_ui(app: &adw::Application) {
    install_css();

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("Panora")
        .default_width(520)
        .default_height(680)
        .resizable(false)
        .build();

    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.add_css_class("panora-shell");

    let header = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    header.set_margin_top(14);
    header.set_margin_start(16);
    header.set_margin_end(16);
    header.set_margin_bottom(10);

    let title_box = gtk::Box::new(gtk::Orientation::Vertical, 2);
    let title = gtk::Label::new(Some("Panora"));
    title.add_css_class("title-2");
    title.set_xalign(0.0);
    let subtitle = gtk::Label::new(Some("Kopyaladıkların burada, güvenli ve hızlı"));
    subtitle.add_css_class("dim-label");
    subtitle.set_xalign(0.0);
    title_box.append(&title);
    title_box.append(&subtitle);
    header.append(&title_box);

    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    header.append(&spacer);

    let private = gtk::ToggleButton::with_label("Özel");
    private.add_css_class("pill");
    header.append(&private);
    let clear = gtk::Button::with_label("Temizle");
    clear.add_css_class("flat");
    header.append(&clear);
    root.append(&header);

    let search = gtk::SearchEntry::new();
    search.set_placeholder_text(Some("Panoda ara…  Ctrl+F"));
    search.set_margin_start(16);
    search.set_margin_end(16);
    search.set_margin_bottom(12);
    search.add_css_class("search-entry");
    root.append(&search);

    let status = gtk::Label::new(Some("Hazırlanıyor…"));
    status.set_xalign(0.0);
    status.set_margin_start(18);
    status.set_margin_end(18);
    status.set_margin_bottom(8);
    status.add_css_class("dim-label");
    root.append(&status);

    let flow = gtk::FlowBox::new();
    flow.set_selection_mode(gtk::SelectionMode::None);
    flow.set_homogeneous(true);
    flow.set_row_spacing(10);
    flow.set_column_spacing(10);
    flow.set_margin_start(16);
    flow.set_margin_end(16);
    flow.set_margin_bottom(12);
    flow.set_max_children_per_line(2);
    flow.set_min_children_per_line(2);

    let scroller = gtk::ScrolledWindow::builder()
        .vexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&flow)
        .build();
    root.append(&scroller);

    let footer = gtk::Label::new(Some(
        "↑ ↓ seç  •  Enter panoya koy  •  Ctrl+F ara  •  Esc kapat",
    ));
    footer.set_margin_top(4);
    footer.set_margin_bottom(12);
    footer.add_css_class("dim-label");
    root.append(&footer);

    window.set_content(Some(&root));

    let refresh = {
        let flow = flow.clone();
        let status = status.clone();
        move |query: String| refresh_cards(&flow, &status, query)
    };

    refresh(String::new());

    {
        let flow = flow.clone();
        let status = status.clone();
        search.connect_search_changed(move |entry| {
            refresh_cards(&flow, &status, entry.text().to_string());
        });
    }

    {
        let flow = flow.clone();
        let status = status.clone();
        clear.connect_clicked(move |_| {
            if call(Request::Clear).is_ok() {
                refresh_cards(&flow, &status, String::new());
            }
        });
    }

    private.connect_toggled(|button| {
        let _ = call(Request::SetPrivate {
            enabled: button.is_active(),
        });
    });

    window.present();
    search.grab_focus();
}

fn refresh_cards(flow: &gtk::FlowBox, status: &gtk::Label, query: String) {
    while let Some(child) = flow.first_child() {
        flow.remove(&child);
    }

    let entries = match call(Request::List(QueryRequest {
        search: (!query.trim().is_empty()).then_some(query),
        limit: 40,
        ..Default::default()
    })) {
        Ok(Response::Success(ResponseData::Entries(entries))) => entries,
        Ok(Response::Failure { message }) => {
            status.set_text(&format!("Daemon hatası: {message}"));
            return;
        }
        Err(error) => {
            status.set_text(&format!("Daemon bekleniyor: {error}"));
            return;
        }
        _ => Vec::new(),
    };

    status.set_text(&format!(
        "{} kayıt  •  seçilen içerik şifreli depodan okunur",
        entries.len()
    ));
    for entry in entries {
        flow.insert(&build_card(entry), -1);
    }
}

fn build_card(entry: Entry) -> gtk::FlowBoxChild {
    let card = gtk::Box::new(gtk::Orientation::Vertical, 8);
    card.add_css_class("history-card");
    card.set_margin_top(2);
    card.set_margin_bottom(2);
    card.set_margin_start(2);
    card.set_margin_end(2);

    let top = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let badge = gtk::Label::new(Some(kind_label(entry.kind)));
    badge.add_css_class("kind-badge");
    top.append(&badge);
    let mime = gtk::Label::new(Some(&entry.primary_mime));
    mime.add_css_class("dim-label");
    mime.set_ellipsize(gtk::pango::EllipsizeMode::End);
    top.append(&mime);
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    top.append(&spacer);
    if entry.pinned {
        let pin = gtk::Label::new(Some("●"));
        pin.add_css_class("pin-dot");
        top.append(&pin);
    }
    card.append(&top);

    if entry.kind == ContentKind::Image {
        let picture = gtk::Picture::new();
        picture.set_size_request(190, 108);
        picture.set_can_shrink(true);
        picture.add_css_class("image-preview");
        card.append(&picture);
        load_image_preview_async(entry.id, picture);
    } else if entry.kind == ContentKind::Color {
        let swatch = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        swatch.set_size_request(-1, 42);
        swatch.add_css_class("color-swatch");
        card.append(&swatch);
        card.append(&preview_label(&entry.preview));
    } else {
        card.append(&preview_label(&entry.preview));
    }

    let footer = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    let size = gtk::Label::new(Some(&format_size(entry.size_bytes)));
    size.add_css_class("dim-label");
    footer.append(&size);
    let footer_spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    footer_spacer.set_hexpand(true);
    footer.append(&footer_spacer);

    let recall = gtk::Button::with_label("Koy");
    recall.add_css_class("suggested-action");
    let id = entry.id;
    recall.connect_clicked(move |_| {
        let _ = call(Request::Recall { id });
    });
    footer.append(&recall);

    let pin = gtk::Button::with_label(if entry.pinned { "Unpin" } else { "Pin" });
    let id = entry.id;
    let pinned = entry.pinned;
    pin.connect_clicked(move |_| {
        let _ = call(Request::Pin {
            id,
            pinned: !pinned,
        });
    });
    footer.append(&pin);

    let delete = gtk::Button::with_label("×");
    delete.add_css_class("flat");
    let id = entry.id;
    delete.connect_clicked(move |_| {
        let _ = call(Request::Delete { id });
    });
    footer.append(&delete);
    card.append(&footer);

    let child = gtk::FlowBoxChild::new();
    child.set_child(Some(&card));
    child
}

fn preview_label(text: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.set_xalign(0.0);
    label.set_wrap(true);
    label.set_lines(4);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    label.add_css_class("preview-text");
    label
}

fn load_image_preview_async(id: i64, picture: gtk::Picture) {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let result = call(Request::Preview { id }).ok();
        let _ = sender.send(result);
    });

    glib::timeout_add_local(Duration::from_millis(20), move || {
        let response = match receiver.try_recv() {
            Ok(response) => response,
            Err(TryRecvError::Empty) => return glib::ControlFlow::Continue,
            Err(TryRecvError::Disconnected) => return glib::ControlFlow::Break,
        };
        let Some(Response::Success(ResponseData::Payloads(payloads))) = response else {
            return glib::ControlFlow::Break;
        };
        let Some(payload) = payloads.into_iter().find(|p| p.mime.starts_with("image/")) else {
            return glib::ControlFlow::Break;
        };
        let loader = PixbufLoader::new();
        loader.set_size(320, 180);
        if loader.write(&payload.data).is_ok() && loader.close().is_ok() {
            if let Some(pixbuf) = loader.pixbuf() {
                let texture = gdk::Texture::for_pixbuf(&pixbuf);
                picture.set_paintable(Some(&texture));
            }
        }
        glib::ControlFlow::Break
    });
}

fn call(request: Request) -> Result<Response, String> {
    let mut stream = UnixStream::connect(socket_path()).map_err(|e| e.to_string())?;
    let data = serde_json::to_vec(&request).map_err(|e| e.to_string())?;
    stream.write_all(&data).map_err(|e| e.to_string())?;
    stream.write_all(b"\n").map_err(|e| e.to_string())?;
    let mut line = String::new();
    BufReader::new(stream)
        .read_line(&mut line)
        .map_err(|e| e.to_string())?;
    serde_json::from_str(line.trim()).map_err(|e| e.to_string())
}

fn kind_label(kind: ContentKind) -> &'static str {
    match kind {
        ContentKind::Text => "METİN",
        ContentKind::RichText => "RICH",
        ContentKind::Link => "LINK",
        ContentKind::Image => "FOTO",
        ContentKind::FileList => "DOSYA",
        ContentKind::Color => "RENK",
        ContentKind::Binary => "BINARY",
    }
}

fn format_size(size: i64) -> String {
    if size < 1024 {
        format!("{size} B")
    } else if size < 1024 * 1024 {
        format!("{:.1} KiB", size as f64 / 1024.0)
    } else {
        format!("{:.1} MiB", size as f64 / 1_048_576.0)
    }
}

fn install_css() {
    let provider = gtk::CssProvider::new();
    provider.load_from_string(
        ".panora-shell { background: #f7f8fa; }
         .history-card { background: #ffffff; border: 1px solid #e2e5ea; border-radius: 14px; padding: 12px; min-height: 118px; }
         .history-card:hover { background: #eef5ff; border-color: #72a9e8; }
         .kind-badge { background: #e7f0ff; color: #1858a8; border-radius: 999px; padding: 3px 7px; font-size: 10px; font-weight: 700; }
         .pin-dot { color: #e09a20; }
         .preview-text { font-size: 13px; color: #1d2733; }
         .image-preview { border-radius: 10px; background: #eef1f5; }
         .color-swatch { background: #36c2ff; border-radius: 10px; }
         .search-entry { min-height: 42px; border-radius: 12px; }
         .pill { border-radius: 999px; }
         .dim-label { color: #728096; font-size: 11px; }
         button { border-radius: 9px; }",
    );
    if let Some(display) = gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}
