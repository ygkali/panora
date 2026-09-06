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
use std::cell::{Cell, RefCell};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::rc::Rc;
use std::sync::mpsc::{self, TryRecvError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Debounce window for search keystrokes, so typing does not hammer the daemon.
const SEARCH_DEBOUNCE_MS: u64 = 120;
/// Page size for the history query.
const PAGE_LIMIT: usize = 60;
/// IPC read/write timeout; the daemon is local, so this only guards hangs.
const IPC_TIMEOUT: Duration = Duration::from_secs(5);
/// Upper bound on one daemon reply. Image previews are large, so this is
/// generous; it exists only to keep the allocation finite.
const MAX_RESPONSE_BYTES: u64 = 128 * 1024 * 1024;

#[derive(Debug, Serialize)]
#[serde(tag = "method", content = "params")]
enum Request {
    List(QueryRequest),
    Recall { id: i64 },
    Pin { id: i64, pinned: bool },
    Delete { id: i64 },
    Clear,
    SetPrivate { enabled: bool },
    Status,
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
    Count(usize),
    Status(StatusData),
    Payloads(Vec<MimePayload>),
    Empty,
}

#[derive(Debug, Deserialize)]
struct StatusData {
    #[allow(dead_code)]
    backend: String,
    #[allow(dead_code)]
    entries: i64,
    private_mode: bool,
    #[allow(dead_code)]
    sync_active: bool,
}

/// Content-kind filter behind the chip row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Filter {
    All,
    Pinned,
    Text,
    Link,
    Image,
    Files,
}

impl Filter {
    /// Chip caption.
    fn label(self) -> &'static str {
        match self {
            Filter::All => "Tümü",
            Filter::Pinned => "Sabitli",
            Filter::Text => "Metin",
            Filter::Link => "Bağlantı",
            Filter::Image => "Görsel",
            Filter::Files => "Dosya",
        }
    }

    /// Content kind sent to the daemon, if this chip narrows by kind.
    fn kind(self) -> Option<&'static str> {
        match self {
            Filter::Text => Some("text"),
            Filter::Link => Some("link"),
            Filter::Image => Some("image"),
            Filter::Files => Some("files"),
            _ => None,
        }
    }
}

/// Widgets and state shared by every callback.
struct Ui {
    window: adw::ApplicationWindow,
    toasts: adw::ToastOverlay,
    title: adw::WindowTitle,
    banner: adw::Banner,
    search: gtk::SearchEntry,
    flow: gtk::FlowBox,
    stack: gtk::Stack,
    empty: adw::StatusPage,
    error: adw::StatusPage,
    private: gtk::ToggleButton,
    entries: RefCell<Vec<Entry>>,
    filter: Cell<Filter>,
    generation: Cell<u64>,
    /// Set while code (not the user) flips the private toggle.
    suppress_private: Cell<bool>,
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
        .default_width(620)
        .default_height(720)
        .width_request(380)
        .height_request(360)
        .build();

    let title = adw::WindowTitle::new("Panora", "pano geçmişi");
    let header = adw::HeaderBar::builder().title_widget(&title).build();

    let private = gtk::ToggleButton::builder()
        .icon_name("security-high-symbolic")
        .tooltip_text("Özel mod: yeni kopyalar kaydedilmez (Ctrl+Shift+P)")
        .build();
    private.add_css_class("flat");
    header.pack_start(&private);

    let menu_button = gtk::MenuButton::builder()
        .icon_name("open-menu-symbolic")
        .tooltip_text("Menü")
        .build();
    menu_button.add_css_class("flat");
    header.pack_end(&menu_button);

    let search = gtk::SearchEntry::new();
    search.set_placeholder_text(Some("Panoda ara…"));
    search.set_margin_top(6);
    search.set_margin_start(14);
    search.set_margin_end(14);
    search.add_css_class("panora-search");

    let chips = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    chips.set_halign(gtk::Align::Center);
    chips.set_margin_top(10);
    chips.set_margin_start(14);
    chips.set_margin_end(14);

    let banner = adw::Banner::new("Özel mod açık — yeni kopyalar kaydedilmiyor");
    banner.set_revealed(false);

    let flow = gtk::FlowBox::new();
    flow.set_selection_mode(gtk::SelectionMode::Single);
    flow.set_activate_on_single_click(true);
    // Not homogeneous: a single image entry would otherwise stretch every text
    // card to its height, and a one-line clipping would take 168px of a 720px
    // popup. Cards size to their content instead.
    flow.set_homogeneous(false);
    flow.set_valign(gtk::Align::Start);
    flow.set_row_spacing(10);
    flow.set_column_spacing(10);
    flow.set_margin_top(12);
    flow.set_margin_start(14);
    flow.set_margin_end(14);
    flow.set_margin_bottom(14);
    flow.set_min_children_per_line(1);
    flow.set_max_children_per_line(3);

    let scroller = gtk::ScrolledWindow::builder()
        .vexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&flow)
        .build();

    let empty = adw::StatusPage::builder()
        .icon_name("edit-paste-symbolic")
        .title("Pano geçmişi boş")
        .description("Bir şey kopyaladığında burada görünecek.")
        .vexpand(true)
        .build();
    empty.add_css_class("compact");

    let error = adw::StatusPage::builder()
        .icon_name("dialog-warning-symbolic")
        .title("Daemon'a bağlanılamadı")
        .vexpand(true)
        .build();
    error.add_css_class("compact");

    let stack = gtk::Stack::new();
    stack.set_transition_type(gtk::StackTransitionType::Crossfade);
    stack.set_transition_duration(120);
    stack.add_named(&scroller, Some("list"));
    stack.add_named(&empty, Some("empty"));
    stack.add_named(&error, Some("error"));
    stack.set_vexpand(true);

    let hint = gtk::Label::new(Some(
        "↑ ↓ gez  ·  Enter panoya koy  ·  Ctrl+D sabitle  ·  Delete sil  ·  Esc kapat",
    ));
    hint.add_css_class("panora-hint");
    hint.set_margin_top(2);
    hint.set_margin_bottom(8);
    hint.set_wrap(true);
    hint.set_justify(gtk::Justification::Center);

    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.append(&banner);
    content.append(&search);
    content.append(&chips);
    content.append(&stack);
    content.append(&hint);

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&content));

    let toasts = adw::ToastOverlay::new();
    toasts.set_child(Some(&toolbar));
    window.set_content(Some(&toasts));

    let ui = Rc::new(Ui {
        window: window.clone(),
        toasts,
        title,
        banner,
        search: search.clone(),
        flow: flow.clone(),
        stack,
        empty,
        error,
        private: private.clone(),
        entries: RefCell::new(Vec::new()),
        filter: Cell::new(Filter::All),
        generation: Cell::new(0),
        suppress_private: Cell::new(false),
    });

    build_chips(&ui, &chips);
    build_menu(&ui, &menu_button);
    connect_retry(&ui);

    {
        let ui = ui.clone();
        search.connect_search_changed(move |_| schedule_refresh(&ui));
    }

    {
        let ui = ui.clone();
        flow.connect_child_activated(move |_, child| {
            if let Some(entry) = entry_at(&ui, child.index()) {
                recall(&ui, entry.id);
            }
        });
    }

    {
        let ui = ui.clone();
        private.connect_toggled(move |button| {
            if ui.suppress_private.get() {
                return;
            }
            let enabled = button.is_active();
            match call(Request::SetPrivate { enabled }) {
                Ok(Response::Success(_)) => {
                    ui.banner.set_revealed(enabled);
                    toast(
                        &ui,
                        if enabled {
                            "Özel mod açıldı — kayıt durduruldu"
                        } else {
                            "Özel mod kapatıldı"
                        },
                    );
                }
                _ => {
                    ui.suppress_private.set(true);
                    button.set_active(!enabled);
                    ui.suppress_private.set(false);
                    toast(&ui, "Özel mod değiştirilemedi");
                }
            }
        });
    }

    install_shortcuts(&ui);
    sync_private_state(&ui);
    refresh(&ui);

    window.present();
    search.grab_focus();
}

/// Build the content-kind chip row as a single-choice group.
fn build_chips(ui: &Rc<Ui>, container: &gtk::Box) {
    let filters = [
        Filter::All,
        Filter::Pinned,
        Filter::Text,
        Filter::Link,
        Filter::Image,
        Filter::Files,
    ];
    let mut group: Option<gtk::ToggleButton> = None;
    for filter in filters {
        let chip = gtk::ToggleButton::with_label(filter.label());
        chip.add_css_class("panora-chip");
        if let Some(first) = &group {
            chip.set_group(Some(first));
        } else {
            group = Some(chip.clone());
        }
        chip.set_active(filter == Filter::All);
        let ui = ui.clone();
        chip.connect_toggled(move |button| {
            if !button.is_active() || ui.filter.get() == filter {
                return;
            }
            ui.filter.set(filter);
            refresh(&ui);
        });
        container.append(&chip);
    }
}

/// Primary menu: destructive clear plus a shortcut cheat sheet.
fn build_menu(ui: &Rc<Ui>, button: &gtk::MenuButton) {
    let popover = gtk::Popover::new();
    let list = gtk::Box::new(gtk::Orientation::Vertical, 2);
    list.set_margin_top(6);
    list.set_margin_bottom(6);
    list.set_margin_start(6);
    list.set_margin_end(6);

    let shortcuts = gtk::Button::builder()
        .child(&menu_label("Klavye kısayolları"))
        .halign(gtk::Align::Fill)
        .build();
    shortcuts.add_css_class("flat");
    {
        let ui = ui.clone();
        let popover = popover.clone();
        shortcuts.connect_clicked(move |_| {
            popover.popdown();
            show_shortcuts(&ui);
        });
    }
    list.append(&shortcuts);

    let clear = gtk::Button::builder()
        .child(&menu_label("Geçmişi temizle"))
        .halign(gtk::Align::Fill)
        .build();
    clear.add_css_class("flat");
    clear.add_css_class("panora-destructive");
    {
        let ui = ui.clone();
        let popover = popover.clone();
        clear.connect_clicked(move |_| {
            popover.popdown();
            confirm_clear(&ui);
        });
    }
    list.append(&clear);

    popover.set_child(Some(&list));
    button.set_popover(Some(&popover));
}

/// Left-aligned label for a flat menu row.
fn menu_label(text: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.set_xalign(0.0);
    label
}

/// Ask before wiping the history; clearing is not undoable.
fn confirm_clear(ui: &Rc<Ui>) {
    let dialog = adw::AlertDialog::new(
        Some("Geçmiş temizlensin mi?"),
        Some(
            "Sabitlenmemiş kayıtlar kalıcı olarak silinir; sabitlediklerin kalır. \
             Bu işlem geri alınamaz.",
        ),
    );
    dialog.add_response("cancel", "Vazgeç");
    dialog.add_response("clear", "Temizle");
    dialog.set_response_appearance("clear", adw::ResponseAppearance::Destructive);
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");

    let handler = ui.clone();
    dialog.connect_response(Some("clear"), move |_, _| match call(Request::Clear) {
        Ok(Response::Success(ResponseData::Count(count))) => {
            toast(&handler, &format!("{count} kayıt silindi"));
            refresh(&handler);
        }
        Ok(Response::Success(_)) => {
            toast(&handler, "Geçmiş temizlendi");
            refresh(&handler);
        }
        _ => toast(&handler, "Geçmiş temizlenemedi"),
    });
    dialog.present(Some(&ui.window));
}

/// Shortcut cheat sheet reachable from the primary menu.
fn show_shortcuts(ui: &Rc<Ui>) {
    let dialog = adw::AlertDialog::new(
        Some("Klavye kısayolları"),
        Some(
            "Ctrl+F  ·  aramaya git\n\
             ↑ ↓ ← →  ·  kayıtlar arasında gez\n\
             Enter  ·  seçili kaydı panoya koy\n\
             Ctrl+D  ·  seçili kaydı sabitle / çöz\n\
             Delete  ·  seçili kaydı sil\n\
             Ctrl+Shift+P  ·  özel modu aç / kapat\n\
             Esc  ·  pencereyi kapat",
        ),
    );
    dialog.add_response("ok", "Tamam");
    dialog.set_default_response(Some("ok"));
    dialog.set_close_response("ok");
    dialog.present(Some(&ui.window));
}

/// Retry button on the daemon-error page.
fn connect_retry(ui: &Rc<Ui>) {
    let retry = gtk::Button::with_label("Yeniden dene");
    retry.add_css_class("pill");
    retry.add_css_class("suggested-action");
    retry.set_halign(gtk::Align::Center);
    let ui_ref = ui.clone();
    retry.connect_clicked(move |_| {
        sync_private_state(&ui_ref);
        refresh(&ui_ref);
    });
    ui.error.set_child(Some(&retry));
}

/// Window-level keyboard handling. Runs in the capture phase so it wins
/// over the focused widget for the few keys Panora owns.
fn install_shortcuts(ui: &Rc<Ui>) {
    let controller = gtk::EventControllerKey::new();
    controller.set_propagation_phase(gtk::PropagationPhase::Capture);
    // Taken before the closure consumes its own handle to the Ui.
    let window = ui.window.clone();
    let ui = ui.clone();
    controller.connect_key_pressed(move |_, key, _, state| {
        let ctrl = state.contains(gdk::ModifierType::CONTROL_MASK);
        let shift = state.contains(gdk::ModifierType::SHIFT_MASK);
        let is_delete = key == gdk::Key::Delete || key == gdk::Key::KP_Delete;
        let is_enter = key == gdk::Key::Return || key == gdk::Key::KP_Enter;

        if key == gdk::Key::Escape {
            // First Escape drops an active search, the second closes the popup.
            if ui.search.text().is_empty() {
                ui.window.close();
            } else {
                ui.search.set_text("");
                ui.search.grab_focus();
            }
            return glib::Propagation::Stop;
        }
        if ctrl && (key == gdk::Key::f || key == gdk::Key::F) {
            ui.search.grab_focus();
            return glib::Propagation::Stop;
        }
        if ctrl && shift && (key == gdk::Key::p || key == gdk::Key::P) {
            ui.private.set_active(!ui.private.is_active());
            return glib::Propagation::Stop;
        }
        if ctrl && (key == gdk::Key::d || key == gdk::Key::D) {
            if let Some(entry) = selected_entry(&ui) {
                toggle_pin(&ui, entry.id, entry.pinned);
            }
            return glib::Propagation::Stop;
        }
        if is_delete {
            // The search entry owns Delete while it is being edited.
            if search_focused(&ui) && !ui.search.text().is_empty() {
                return glib::Propagation::Proceed;
            }
            if let Some(entry) = selected_entry(&ui) {
                delete_entry(&ui, entry.id);
            }
            return glib::Propagation::Stop;
        }
        if (key == gdk::Key::Down || is_enter) && search_focused(&ui) {
            // Step out of the search box into the results grid.
            let Some(child) = ui.flow.child_at_index(0) else {
                return glib::Propagation::Proceed;
            };
            // Focus first, then select: GtkFlowBox resets the selection while
            // it moves its cursor, so selecting before grab_focus() leaves the
            // grid focused with nothing selected, and every shortcut that
            // relies on the selection silently does nothing.
            child.grab_focus();
            ui.flow.select_child(&child);
            if is_enter {
                if let Some(entry) = entry_at(&ui, child.index()) {
                    recall(&ui, entry.id);
                }
            }
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    window.add_controller(controller);
}

/// Read the daemon's private-mode flag so the toggle reflects reality.
fn sync_private_state(ui: &Rc<Ui>) {
    if let Ok(Response::Success(ResponseData::Status(status))) = call(Request::Status) {
        ui.suppress_private.set(true);
        ui.private.set_active(status.private_mode);
        ui.suppress_private.set(false);
        ui.banner.set_revealed(status.private_mode);
    }
}

/// Coalesce search keystrokes into one query.
fn schedule_refresh(ui: &Rc<Ui>) {
    let generation = ui.generation.get().wrapping_add(1);
    ui.generation.set(generation);
    let ui = ui.clone();
    glib::timeout_add_local_once(Duration::from_millis(SEARCH_DEBOUNCE_MS), move || {
        if ui.generation.get() == generation {
            refresh(&ui);
        }
    });
}

/// Query the daemon and rebuild the grid, empty state or error state.
fn refresh(ui: &Rc<Ui>) {
    while let Some(child) = ui.flow.first_child() {
        ui.flow.remove(&child);
    }
    ui.entries.borrow_mut().clear();

    let query = ui.search.text().trim().to_string();
    let filter = ui.filter.get();
    let search_term = if query.is_empty() {
        None
    } else {
        Some(query.clone())
    };
    let entries = match call(Request::List(QueryRequest {
        search: search_term,
        kind: filter.kind().map(str::to_string),
        pinned_only: filter == Filter::Pinned,
        limit: PAGE_LIMIT,
        offset: 0,
    })) {
        Ok(Response::Success(ResponseData::Entries(entries))) => entries,
        Ok(Response::Failure { message }) => {
            ui.error.set_title("Daemon hata döndürdü");
            // StatusPage descriptions are parsed as Pango markup, so anything
            // that did not come from this file has to be escaped first.
            ui.error
                .set_description(Some(&glib::markup_escape_text(&message)));
            ui.stack.set_visible_child_name("error");
            ui.title.set_subtitle("bağlantı sorunu");
            return;
        }
        Err(error) => {
            ui.error.set_title("Daemon'a bağlanılamadı");
            ui.error.set_description(Some(&format!(
                "panod çalışmıyor olabilir. Başlatmak için:\n\
                 systemctl --user start panod.service\n\n{}",
                glib::markup_escape_text(&error)
            )));
            ui.stack.set_visible_child_name("error");
            ui.title.set_subtitle("bağlantı yok");
            return;
        }
        _ => Vec::new(),
    };

    ui.title.set_subtitle(&subtitle_for(entries.len(), filter));

    if entries.is_empty() {
        if query.is_empty() {
            ui.empty.set_icon_name(Some("edit-paste-symbolic"));
            ui.empty.set_title(if filter == Filter::All {
                "Pano geçmişi boş"
            } else {
                "Bu filtrede kayıt yok"
            });
            ui.empty
                .set_description(Some("Bir şey kopyaladığında burada görünecek."));
        } else {
            ui.empty.set_icon_name(Some("system-search-symbolic"));
            ui.empty.set_title("Sonuç yok");
            // The search term is user input landing in a markup-parsed label.
            ui.empty.set_description(Some(&format!(
                "“{}” ile eşleşen kayıt bulunamadı.",
                glib::markup_escape_text(&query)
            )));
        }
        ui.stack.set_visible_child_name("empty");
        return;
    }

    for entry in &entries {
        let child = build_card(ui, entry);
        ui.flow.insert(&child, -1);
    }
    *ui.entries.borrow_mut() = entries;
    ui.stack.set_visible_child_name("list");
}

/// Human-readable record count for the header subtitle.
fn subtitle_for(count: usize, filter: Filter) -> String {
    match filter {
        Filter::All => format!("{count} kayıt"),
        other => format!("{count} kayıt · {}", other.label().to_lowercase()),
    }
}

fn build_card(ui: &Rc<Ui>, entry: &Entry) -> gtk::FlowBoxChild {
    let card = gtk::Box::new(gtk::Orientation::Vertical, 8);
    card.add_css_class("history-card");

    let top = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let icon = gtk::Image::from_icon_name(kind_icon(entry.kind));
    icon.add_css_class("kind-icon");
    icon.set_pixel_size(14);
    top.append(&icon);

    let badge = gtk::Label::new(Some(kind_label(entry.kind)));
    badge.add_css_class("kind-badge");
    top.append(&badge);

    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    top.append(&spacer);

    let time = gtk::Label::new(Some(&relative_time(entry.last_seen_at)));
    time.add_css_class("card-meta");
    time.set_ellipsize(gtk::pango::EllipsizeMode::End);
    top.append(&time);
    card.append(&top);

    match entry.kind {
        ContentKind::Image => {
            let picture = gtk::Picture::new();
            picture.set_size_request(-1, 116);
            picture.set_can_shrink(true);
            picture.set_content_fit(gtk::ContentFit::Cover);
            picture.add_css_class("image-preview");
            card.append(&picture);
            load_image_preview_async(entry.id, picture);
        }
        ContentKind::Color => {
            card.append(&color_swatch(&entry.preview));
            card.append(&preview_label(&entry.preview, 1));
        }
        _ => card.append(&preview_label(&entry.preview, 4)),
    }

    let bottom = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    bottom.set_valign(gtk::Align::End);
    bottom.set_vexpand(true);

    let meta = gtk::Label::new(Some(&card_meta(entry)));
    meta.add_css_class("card-meta");
    meta.set_xalign(0.0);
    meta.set_ellipsize(gtk::pango::EllipsizeMode::End);
    meta.set_hexpand(true);
    bottom.append(&meta);

    let pin = gtk::Button::from_icon_name(if entry.pinned {
        "starred-symbolic"
    } else {
        "non-starred-symbolic"
    });
    pin.add_css_class("flat");
    pin.add_css_class("card-action");
    pin.set_tooltip_text(Some(if entry.pinned {
        "Sabitlemeyi kaldır (Ctrl+D)"
    } else {
        "Sabitle (Ctrl+D)"
    }));
    if entry.pinned {
        pin.add_css_class("pinned");
    }
    {
        let ui = ui.clone();
        let id = entry.id;
        let pinned = entry.pinned;
        pin.connect_clicked(move |_| toggle_pin(&ui, id, pinned));
    }
    bottom.append(&pin);

    let delete = gtk::Button::from_icon_name("user-trash-symbolic");
    delete.add_css_class("flat");
    delete.add_css_class("card-action");
    delete.set_tooltip_text(Some("Sil (Delete)"));
    {
        let ui = ui.clone();
        let id = entry.id;
        delete.connect_clicked(move |_| delete_entry(&ui, id));
    }
    bottom.append(&delete);
    card.append(&bottom);

    let child = gtk::FlowBoxChild::new();
    child.set_child(Some(&card));
    child.set_tooltip_text(Some(&card_tooltip(entry)));
    child
}

/// Bottom-left metadata line: size, and source app when the daemon knows it.
fn card_meta(entry: &Entry) -> String {
    match &entry.source_app {
        Some(app) if !app.is_empty() => format!("{} · {}", format_size(entry.size_bytes), app),
        _ => format_size(entry.size_bytes),
    }
}

/// Hover tooltip: the full MIME type plus the primary action reminder.
fn card_tooltip(entry: &Entry) -> String {
    format!("{}\nPanoya koymak için tıkla", entry.primary_mime)
}

/// Draw the parsed color instead of a fixed placeholder swatch.
fn color_swatch(preview: &str) -> gtk::DrawingArea {
    let rgba =
        gdk::RGBA::parse(preview.trim()).unwrap_or_else(|_| gdk::RGBA::new(0.5, 0.5, 0.5, 1.0));
    let area = gtk::DrawingArea::new();
    area.set_content_height(48);
    area.add_css_class("color-swatch");
    area.set_draw_func(move |_, cr, width, height| {
        let (w, h) = (f64::from(width), f64::from(height));
        let radius = 10.0_f64.min(w / 2.0).min(h / 2.0);
        rounded_rect(cr, w, h, radius);
        cr.set_source_rgba(
            f64::from(rgba.red()),
            f64::from(rgba.green()),
            f64::from(rgba.blue()),
            f64::from(rgba.alpha()),
        );
        let _ = cr.fill_preserve();
        cr.set_source_rgba(0.0, 0.0, 0.0, 0.14);
        cr.set_line_width(1.0);
        let _ = cr.stroke();
    });
    area
}

/// Append a rounded rectangle covering the whole drawing area.
fn rounded_rect(cr: &gtk::cairo::Context, w: f64, h: f64, r: f64) {
    let pi = std::f64::consts::PI;
    cr.new_sub_path();
    cr.arc(w - r, r, r, -pi / 2.0, 0.0);
    cr.arc(w - r, h - r, r, 0.0, pi / 2.0);
    cr.arc(r, h - r, r, pi / 2.0, pi);
    cr.arc(r, r, r, pi, 1.5 * pi);
    cr.close_path();
}

fn preview_label(text: &str, lines: i32) -> gtk::Label {
    let label = gtk::Label::new(Some(text.trim()));
    label.set_xalign(0.0);
    label.set_yalign(0.0);
    label.set_wrap(true);
    label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    label.set_lines(lines);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    label.add_css_class("preview-text");
    label
}

/// Put an entry back on the clipboard and dismiss the popup, Win+V style.
fn recall(ui: &Rc<Ui>, id: i64) {
    match call(Request::Recall { id }) {
        Ok(Response::Success(_)) => ui.window.close(),
        _ => toast(ui, "Panoya konulamadı"),
    }
}

fn toggle_pin(ui: &Rc<Ui>, id: i64, pinned: bool) {
    match call(Request::Pin {
        id,
        pinned: !pinned,
    }) {
        Ok(Response::Success(_)) => {
            toast(
                ui,
                if pinned {
                    "Sabitleme kaldırıldı"
                } else {
                    "Sabitlendi"
                },
            );
            refresh(ui);
        }
        _ => toast(ui, "Sabitleme değiştirilemedi"),
    }
}

fn delete_entry(ui: &Rc<Ui>, id: i64) {
    match call(Request::Delete { id }) {
        Ok(Response::Success(_)) => {
            toast(ui, "Kayıt silindi");
            refresh(ui);
        }
        _ => toast(ui, "Kayıt silinemedi"),
    }
}

fn toast(ui: &Rc<Ui>, message: &str) {
    ui.toasts
        .add_toast(adw::Toast::builder().title(message).timeout(2).build());
}

/// The entry backing a flow-box position, if the grid is still in sync.
fn entry_at(ui: &Rc<Ui>, index: i32) -> Option<Entry> {
    let index = usize::try_from(index).ok()?;
    ui.entries.borrow().get(index).cloned()
}

/// True when the keyboard focus is inside the search box.
///
/// `search.has_focus()` alone is always false there: GtkSearchEntry is a
/// composite widget and the focus actually lands on its inner GtkText, so the
/// naive check silently disabled both the search-to-grid hand-off and the
/// guard that keeps Delete editing text instead of deleting an entry.
fn search_focused(ui: &Ui) -> bool {
    ui.search.has_focus() || ui.search.focus_child().is_some()
}

/// The child a keyboard shortcut should act on. Normally that is the selected
/// child, but the grid can hold the cursor with an empty selection, so fall
/// back to whatever currently has focus rather than doing nothing.
fn current_child(ui: &Rc<Ui>) -> Option<gtk::FlowBoxChild> {
    if let Some(child) = ui.flow.selected_children().into_iter().next() {
        return Some(child);
    }
    ui.flow.focus_child()?.downcast::<gtk::FlowBoxChild>().ok()
}

fn selected_entry(ui: &Rc<Ui>) -> Option<Entry> {
    entry_at(ui, current_child(ui)?.index())
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
    let stream = UnixStream::connect(socket_path()).map_err(|e| e.to_string())?;
    stream
        .set_read_timeout(Some(IPC_TIMEOUT))
        .map_err(|e| e.to_string())?;
    stream
        .set_write_timeout(Some(IPC_TIMEOUT))
        .map_err(|e| e.to_string())?;
    let mut writer = &stream;
    let data = serde_json::to_vec(&request).map_err(|e| e.to_string())?;
    writer.write_all(&data).map_err(|e| e.to_string())?;
    writer.write_all(b"\n").map_err(|e| e.to_string())?;
    let mut line = String::new();
    // Bound the reply too. A Preview response carries image bytes as a JSON
    // array, so the cap has to be generous, but it must exist: without it a
    // misbehaving or impersonated daemon could grow this allocation forever.
    BufReader::new(&stream)
        .take(MAX_RESPONSE_BYTES)
        .read_line(&mut line)
        .map_err(|e| e.to_string())?;
    serde_json::from_str(line.trim()).map_err(|e| e.to_string())
}

fn kind_label(kind: ContentKind) -> &'static str {
    match kind {
        ContentKind::Text => "METİN",
        ContentKind::RichText => "BİÇİMLİ",
        ContentKind::Link => "BAĞLANTI",
        ContentKind::Image => "GÖRSEL",
        ContentKind::FileList => "DOSYA",
        ContentKind::Color => "RENK",
        ContentKind::Binary => "İKİLİ",
    }
}

fn kind_icon(kind: ContentKind) -> &'static str {
    match kind {
        ContentKind::Text => "text-x-generic-symbolic",
        ContentKind::RichText => "font-x-generic-symbolic",
        ContentKind::Link => "insert-link-symbolic",
        ContentKind::Image => "image-x-generic-symbolic",
        ContentKind::FileList => "folder-symbolic",
        ContentKind::Color => "color-select-symbolic",
        ContentKind::Binary => "application-x-executable-symbolic",
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

/// Turkish relative timestamp for the card header.
fn relative_time(timestamp: i64) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(timestamp);
    let diff = (now - timestamp).max(0);
    match diff {
        0..=59 => "az önce".to_string(),
        60..=3599 => format!("{} dk önce", diff / 60),
        3600..=86_399 => format!("{} sa önce", diff / 3600),
        86_400..=604_799 => format!("{} gün önce", diff / 86_400),
        _ => format!("{} hafta önce", diff / 604_800),
    }
}

fn install_css() {
    let provider = gtk::CssProvider::new();
    // Colors come from libadwaita's named palette, so light mode, dark mode
    // and the user's accent colour all work from one stylesheet.
    provider.load_from_string(
        "window { background: @window_bg_color; }

         .panora-search { min-height: 38px; border-radius: 12px; }

         .panora-chip {
             border-radius: 999px;
             padding: 2px 12px;
             min-height: 26px;
             font-size: 12px;
             background: alpha(currentColor, 0.07);
             border: none;
             box-shadow: none;
         }
         .panora-chip:hover { background: alpha(currentColor, 0.12); }
         .panora-chip:checked {
             background: @accent_bg_color;
             color: @accent_fg_color;
         }

         flowboxchild { padding: 0; border-radius: 16px; }
         flowboxchild:selected { background: none; }

         .history-card {
             background: @card_bg_color;
             border: 1px solid alpha(@card_fg_color, 0.11);
             border-radius: 16px;
             padding: 12px;
             min-height: 84px;
             transition: border-color 120ms ease, background 120ms ease;
         }
         .history-card:hover {
             border-color: alpha(@accent_bg_color, 0.5);
             background: mix(@card_bg_color, @accent_bg_color, 0.05);
         }
         flowboxchild:selected .history-card {
             border-color: @accent_bg_color;
             background: mix(@card_bg_color, @accent_bg_color, 0.1);
             box-shadow: 0 0 0 1px @accent_bg_color;
         }

         .kind-badge {
             font-size: 9px;
             font-weight: 800;
             letter-spacing: 0.6px;
             color: alpha(@card_fg_color, 0.62);
         }
         .kind-icon { color: alpha(@card_fg_color, 0.55); }

         .preview-text { font-size: 13px; color: @card_fg_color; }
         .card-meta { font-size: 10px; color: alpha(@card_fg_color, 0.5); }

         .card-action {
             min-width: 24px;
             min-height: 24px;
             padding: 2px;
             opacity: 0.55;
         }
         .card-action:hover { opacity: 1; }
         .card-action.pinned { color: @accent_color; opacity: 1; }

         .image-preview { border-radius: 10px; background: alpha(@card_fg_color, 0.06); }
         .color-swatch { border-radius: 10px; }

         .panora-hint { font-size: 10px; color: alpha(@window_fg_color, 0.45); }
         .panora-destructive { color: @error_color; }",
    );
    if let Some(display) = gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}
