// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! The history popup: search, filter chips, card grid, keyboard handling.

use crate::util::{call, format_size, kind_icon, kind_label, relative_time};
use crate::{details, settings, App};
use gdk_pixbuf::PixbufLoader;
use gtk::gdk;
use gtk4 as gtk;
use libadwaita as adw;
use libadwaita::prelude::*;
use panora_core::i18n::{fill, Strings};
use panora_core::ipc::{QueryRequest, Request, ResponseData};
use panora_core::model::{ContentKind, Entry};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::mpsc::{self, TryRecvError};
use std::time::Duration;

/// Debounce window for search keystrokes, so typing does not hammer the daemon.
const SEARCH_DEBOUNCE_MS: u64 = 120;
/// Page size for the history query.
const PAGE_LIMIT: usize = 60;
/// How often the popup asks the daemon whether history changed.
const LIVE_REFRESH_MS: u64 = 1500;

/// Content-kind filter behind the chip row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Filter {
    All,
    Pinned,
    Text,
    Link,
    Image,
    Files,
    RichText,
    Color,
}

impl Filter {
    const ALL: [Filter; 8] = [
        Filter::All,
        Filter::Pinned,
        Filter::Text,
        Filter::Link,
        Filter::Image,
        Filter::Files,
        Filter::RichText,
        Filter::Color,
    ];

    /// Chip caption.
    fn label(self, s: &Strings) -> &'static str {
        match self {
            Filter::All => s.filter_all,
            Filter::Pinned => s.filter_pinned,
            Filter::Text => s.filter_text,
            Filter::Link => s.filter_link,
            Filter::Image => s.filter_image,
            Filter::Files => s.filter_files,
            Filter::RichText => s.filter_richtext,
            Filter::Color => s.filter_color,
        }
    }

    /// Content kind sent to the daemon, if this chip narrows by kind.
    fn kind(self) -> Option<&'static str> {
        match self {
            Filter::Text => Some("text"),
            Filter::Link => Some("link"),
            Filter::Image => Some("image"),
            Filter::Files => Some("files"),
            Filter::RichText => Some("richtext"),
            Filter::Color => Some("color"),
            _ => None,
        }
    }
}

/// Widgets and state shared by every callback.
pub struct Ui {
    pub app: Rc<App>,
    pub s: &'static Strings,
    pub window: adw::ApplicationWindow,
    pub toasts: adw::ToastOverlay,
    title: adw::WindowTitle,
    banner: adw::Banner,
    search: gtk::SearchEntry,
    flow: gtk::FlowBox,
    stack: gtk::Stack,
    empty: adw::StatusPage,
    error: adw::StatusPage,
    private: gtk::ToggleButton,
    load_more: gtk::Button,
    entries: RefCell<Vec<Entry>>,
    filter: Cell<Filter>,
    generation: Cell<u64>,
    /// Set while code (not the user) flips the private toggle.
    suppress_private: Cell<bool>,
    /// Daemon revision the grid currently shows.
    revision: Cell<u64>,
    /// No further pages after the last one fetched.
    exhausted: Cell<bool>,
    /// The window has held keyboard focus at least once; only then does a
    /// focus loss mean "the user went elsewhere".
    was_active: Cell<bool>,
}

/// How many rows get a Ctrl+N shortcut and show its number.
const QUICK_PICK_ROWS: usize = 9;
/// Rows a PageUp/PageDown press moves the selection by.
const PAGE_STEP: i32 = 8;

/// Build and present the popup.
pub fn build(app: &adw::Application, state: &Rc<App>) {
    let s = state.strings;
    // A clipboard picker is a panel, not a document window: one column of
    // full-width rows, sized like the Windows Win+V flyout so the eye travels
    // straight down the history instead of scanning a grid.
    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title(s.app_name)
        .default_width(420)
        .default_height(660)
        .width_request(340)
        .height_request(420)
        .build();

    let title = adw::WindowTitle::new(s.app_name, s.subtitle_history);
    let header = adw::HeaderBar::builder().title_widget(&title).build();

    let private = gtk::ToggleButton::builder()
        .icon_name("security-high-symbolic")
        .tooltip_text(s.private_tooltip)
        .build();
    private.add_css_class("flat");
    name_for_a11y(&private, s.private_label);
    header.pack_start(&private);

    let menu_button = gtk::MenuButton::builder()
        .icon_name("open-menu-symbolic")
        .tooltip_text(s.menu_tooltip)
        .build();
    menu_button.add_css_class("flat");
    name_for_a11y(&menu_button, s.menu_tooltip);
    header.pack_end(&menu_button);

    // "Clear all" is one of the two things anyone opens a clipboard history
    // for; it belongs in reach, not three clicks down a menu.
    let clear = gtk::Button::builder()
        .icon_name("user-trash-symbolic")
        .tooltip_text(s.menu_clear)
        .build();
    clear.add_css_class("flat");
    name_for_a11y(&clear, s.menu_clear);
    header.pack_end(&clear);

    let search = gtk::SearchEntry::new();
    search.set_placeholder_text(Some(s.search_placeholder));
    search.set_margin_top(6);
    search.set_margin_start(14);
    search.set_margin_end(14);
    search.add_css_class("panora-search");

    // A FlowBox, not a Box in a scroller: eight chips do not fit a panel this
    // narrow, and the row used to clip the last two with nothing to say they
    // were there. Wrapping also survives translation -- the same eight labels
    // are markedly longer in Turkish or German than in English.
    let chips = gtk::FlowBox::new();
    chips.set_selection_mode(gtk::SelectionMode::None);
    chips.set_homogeneous(false);
    chips.set_row_spacing(6);
    chips.set_column_spacing(6);
    chips.set_min_children_per_line(1);
    chips.set_max_children_per_line(Filter::ALL.len() as u32);
    // Fill, not Start: with Start the flow box is allocated only its minimum
    // width (one chip), wraps every chip onto its own line and, because its
    // height was requested for the full width, overflows into the list below.
    // A filled flow box still lays chips out from the leading edge, so RTL
    // mirroring comes from GTK's text direction, not from the alignment.
    chips.set_halign(gtk::Align::Fill);
    chips.set_margin_top(10);
    chips.set_margin_start(14);
    chips.set_margin_end(14);

    let banner = adw::Banner::new(s.banner_private);
    banner.set_revealed(false);

    let flow = gtk::FlowBox::new();
    flow.set_selection_mode(gtk::SelectionMode::Single);
    flow.set_activate_on_single_click(true);
    // Not homogeneous: a single image entry would otherwise stretch every text
    // card to its height. Cards size to their content instead.
    flow.set_homogeneous(false);
    flow.set_valign(gtk::Align::Start);
    flow.set_row_spacing(6);
    flow.set_column_spacing(0);
    flow.set_margin_top(10);
    flow.set_margin_start(12);
    flow.set_margin_end(12);
    flow.set_margin_bottom(6);
    // Exactly one column. A grid makes "most recent" a corner instead of the
    // top of a list, and Up/Down then skip past entries a row at a time.
    flow.set_min_children_per_line(1);
    flow.set_max_children_per_line(1);

    let load_more = gtk::Button::with_label(s.load_more);
    load_more.add_css_class("pill");
    load_more.set_halign(gtk::Align::Center);
    load_more.set_margin_top(4);
    load_more.set_margin_bottom(14);
    load_more.set_visible(false);

    let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
    list.append(&flow);
    list.append(&load_more);

    let scroller = gtk::ScrolledWindow::builder()
        .vexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&list)
        .build();

    let empty = adw::StatusPage::builder()
        .icon_name("edit-paste-symbolic")
        .title(s.empty_title)
        .description(s.empty_description)
        .vexpand(true)
        .build();
    empty.add_css_class("compact");

    let error = adw::StatusPage::builder()
        .icon_name("dialog-warning-symbolic")
        .title(s.error_connect_title)
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

    // libadwaita's own type classes instead of a hand-set pixel size, so the
    // line follows the user's text-scaling factor like every other label.
    let hint = gtk::Label::new(Some(s.hint_line));
    hint.add_css_class("caption");
    hint.add_css_class("dim-label");
    hint.set_margin_top(4);
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
        app: state.clone(),
        s,
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
        load_more: load_more.clone(),
        entries: RefCell::new(Vec::new()),
        filter: Cell::new(Filter::All),
        generation: Cell::new(0),
        suppress_private: Cell::new(false),
        revision: Cell::new(0),
        exhausted: Cell::new(true),
        was_active: Cell::new(false),
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
        load_more.connect_clicked(move |_| load_next_page(&ui));
    }
    {
        let ui = ui.clone();
        clear.connect_clicked(move |_| confirm_clear(&ui));
    }
    {
        let ui = ui.clone();
        scroller.connect_edge_reached(move |_, position| {
            if position == gtk::PositionType::Bottom {
                load_next_page(&ui);
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
            match call(&Request::SetPrivate { enabled }) {
                Ok(_) => {
                    ui.banner.set_revealed(enabled);
                    toast(
                        &ui,
                        if enabled {
                            ui.s.toast_private_on
                        } else {
                            ui.s.toast_private_off
                        },
                    );
                }
                Err(_) => {
                    ui.suppress_private.set(true);
                    button.set_active(!enabled);
                    ui.suppress_private.set(false);
                    toast(&ui, ui.s.toast_private_failed);
                }
            }
        });
    }

    install_shortcuts(&ui);
    sync_status(&ui);
    refresh(&ui);
    start_live_refresh(&ui);

    // Win+V behaviour: the panel goes away when the user switches to
    // another window. It has to have been active once first, so a slow
    // compositor (or a headless run without a window manager) does not
    // close it before it ever had focus; in-window dialogs keep it open,
    // and the hide-before-recall step is not a focus loss either.
    {
        let ui = ui.clone();
        window.connect_is_active_notify(move |window| {
            if window.is_active() {
                ui.was_active.set(true);
                return;
            }
            if ui.was_active.get()
                && ui.app.config.borrow().ui.close_on_focus_loss
                && window.is_visible()
                && window.visible_dialog().is_none()
            {
                window.close();
            }
        });
    }

    window.present();
    search.grab_focus();
}

/// Build the content-kind chip row as a single-choice group.
fn build_chips(ui: &Rc<Ui>, container: &gtk::FlowBox) {
    let mut group: Option<gtk::ToggleButton> = None;
    for filter in Filter::ALL {
        let chip = gtk::ToggleButton::with_label(filter.label(ui.s));
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
        container.insert(&chip, -1);
    }
}

/// Primary menu: shortcuts, settings and the destructive clear.
fn build_menu(ui: &Rc<Ui>, button: &gtk::MenuButton) {
    let popover = gtk::Popover::new();
    let list = gtk::Box::new(gtk::Orientation::Vertical, 2);
    list.set_margin_top(6);
    list.set_margin_bottom(6);
    list.set_margin_start(6);
    list.set_margin_end(6);

    let add_item = |label: &str, destructive: bool, action: Box<dyn Fn()>| {
        let item = gtk::Button::builder()
            .child(&menu_label(label))
            .halign(gtk::Align::Fill)
            .build();
        item.add_css_class("flat");
        if destructive {
            item.add_css_class("panora-destructive");
        }
        let popover = popover.clone();
        item.connect_clicked(move |_| {
            popover.popdown();
            action();
        });
        list.append(&item);
    };

    {
        let ui = ui.clone();
        add_item(
            ui.s.menu_shortcuts,
            false,
            Box::new(move || show_shortcuts(&ui)),
        );
    }
    {
        let ui = ui.clone();
        add_item(
            ui.s.menu_settings,
            false,
            Box::new(move || settings::show(&ui)),
        );
    }
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
    let dialog = adw::AlertDialog::new(Some(ui.s.clear_title), Some(ui.s.clear_body));
    dialog.add_response("cancel", ui.s.cancel);
    dialog.add_response("clear", ui.s.clear);
    dialog.set_response_appearance("clear", adw::ResponseAppearance::Destructive);
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");

    let handler = ui.clone();
    dialog.connect_response(Some("clear"), move |_, _| match call(&Request::Clear) {
        Ok(ResponseData::Count(count)) => {
            toast(
                &handler,
                &fill(handler.s.toast_cleared_n, "n", &count.to_string()),
            );
            refresh(&handler);
        }
        Ok(_) => {
            toast(&handler, handler.s.toast_cleared);
            refresh(&handler);
        }
        Err(_) => toast(&handler, handler.s.toast_clear_failed),
    });
    dialog.present(Some(&ui.window));
}

/// Shortcut cheat sheet reachable from the primary menu.
fn show_shortcuts(ui: &Rc<Ui>) {
    let dialog = adw::AlertDialog::new(Some(ui.s.shortcuts_title), Some(ui.s.shortcuts_body));
    dialog.add_response("ok", ui.s.ok);
    dialog.set_default_response(Some("ok"));
    dialog.set_close_response("ok");
    dialog.present(Some(&ui.window));
}

/// Retry button on the daemon-error page.
fn connect_retry(ui: &Rc<Ui>) {
    let retry = gtk::Button::with_label(ui.s.retry);
    retry.add_css_class("pill");
    retry.add_css_class("suggested-action");
    retry.set_halign(gtk::Align::Center);
    let ui_ref = ui.clone();
    retry.connect_clicked(move |_| {
        sync_status(&ui_ref);
        refresh(&ui_ref);
    });
    ui.error.set_child(Some(&retry));
}

/// Window-level keyboard handling. Runs in the capture phase so it wins
/// over the focused widget for the few keys Panora owns.
fn install_shortcuts(ui: &Rc<Ui>) {
    let controller = gtk::EventControllerKey::new();
    controller.set_propagation_phase(gtk::PropagationPhase::Capture);
    let window = ui.window.clone();
    let ui = ui.clone();
    controller.connect_key_pressed(move |_, key, _, state| {
        // Adwaita dialogs render inside this window, so the capture-phase
        // handler would otherwise steal Escape/Delete/Space from them.
        if ui.window.visible_dialog().is_some() {
            return glib::Propagation::Proceed;
        }
        let ctrl = state.contains(gdk::ModifierType::CONTROL_MASK);
        let shift = state.contains(gdk::ModifierType::SHIFT_MASK);
        let alt = state.contains(gdk::ModifierType::ALT_MASK);
        let is_delete = key == gdk::Key::Delete || key == gdk::Key::KP_Delete;
        let is_enter = key == gdk::Key::Return || key == gdk::Key::KP_Enter;

        // Ctrl+1 … Ctrl+9: the Nth row without walking to it.
        if ctrl && !shift {
            if let Some(n) = key.to_unicode().and_then(|c| c.to_digit(10)) {
                if (1..=QUICK_PICK_ROWS as u32).contains(&n) {
                    if let Some(entry) = entry_at(&ui, n as i32 - 1) {
                        recall_with(&ui, entry.id, None);
                    }
                    return glib::Propagation::Stop;
                }
            }
        }
        // Shift+Enter: the plain text of the selected (or first) row only.
        if is_enter && shift {
            let entry = if search_focused(&ui) {
                entry_at(&ui, 0)
            } else {
                selected_entry(&ui)
            };
            if let Some(entry) = entry {
                recall_with(&ui, entry.id, Some("text/plain"));
            }
            return glib::Propagation::Stop;
        }
        if !search_focused(&ui)
            && matches!(
                key,
                gdk::Key::Home | gdk::Key::End | gdk::Key::Page_Up | gdk::Key::Page_Down
            )
        {
            let count = ui.entries.borrow().len() as i32;
            if count > 0 {
                let current = current_child(&ui).map(|c| c.index()).unwrap_or(0);
                let target = match key {
                    gdk::Key::Home => 0,
                    gdk::Key::End => count - 1,
                    gdk::Key::Page_Up => (current - PAGE_STEP).max(0),
                    _ => (current + PAGE_STEP).min(count - 1),
                };
                if let Some(child) = ui.flow.child_at_index(target) {
                    child.grab_focus();
                    ui.flow.select_child(&child);
                }
            }
            return glib::Propagation::Stop;
        }

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
        if ctrl && key == gdk::Key::comma {
            settings::show(&ui);
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
        if key == gdk::Key::space && !search_focused(&ui) {
            if let Some(entry) = selected_entry(&ui) {
                details::show(&ui, &entry);
                return glib::Propagation::Stop;
            }
        }
        if (key == gdk::Key::Down || is_enter) && search_focused(&ui) {
            // Step out of the search box into the results grid.
            let Some(child) = ui.flow.child_at_index(0) else {
                return glib::Propagation::Proceed;
            };
            // Focus first, then select: GtkFlowBox resets the selection while
            // it moves its cursor, so selecting before grab_focus() leaves the
            // grid focused with nothing selected.
            child.grab_focus();
            ui.flow.select_child(&child);
            if is_enter {
                if let Some(entry) = entry_at(&ui, child.index()) {
                    recall(&ui, entry.id);
                }
            }
            return glib::Propagation::Stop;
        }
        // Typing while the list has focus starts a search, so the panel
        // behaves the same whichever widget happens to hold the cursor.
        if !search_focused(&ui) && !ctrl && !alt {
            if let Some(ch) = key
                .to_unicode()
                .filter(|c| !c.is_control() && !c.is_whitespace())
            {
                let mut text = ui.search.text().to_string();
                text.push(ch);
                ui.search.grab_focus();
                ui.search.set_text(&text);
                ui.search.set_position(-1);
                return glib::Propagation::Stop;
            }
        }
        glib::Propagation::Proceed
    });
    window.add_controller(controller);
}

/// Read the daemon's private-mode flag and revision so the UI reflects reality.
fn sync_status(ui: &Rc<Ui>) -> bool {
    if let Ok(ResponseData::Status(status)) = call(&Request::Status) {
        ui.suppress_private.set(true);
        ui.private.set_active(status.private_mode);
        ui.suppress_private.set(false);
        ui.banner.set_revealed(status.private_mode);
        let changed = ui.revision.get() != status.revision;
        ui.revision.set(status.revision);
        return changed;
    }
    false
}

/// Poll the daemon revision so copies made while the popup is open appear.
fn start_live_refresh(ui: &Rc<Ui>) {
    let ui = ui.clone();
    glib::timeout_add_local(Duration::from_millis(LIVE_REFRESH_MS), move || {
        if !ui.window.is_visible() {
            return glib::ControlFlow::Break;
        }
        if sync_status(&ui) {
            refresh(&ui);
        }
        glib::ControlFlow::Continue
    });
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

fn current_query(ui: &Ui, offset: usize) -> Request {
    let query = ui.search.text().trim().to_string();
    let filter = ui.filter.get();
    Request::List(QueryRequest {
        search: if query.is_empty() { None } else { Some(query) },
        kind: filter.kind().map(str::to_string),
        pinned_only: filter == Filter::Pinned,
        limit: PAGE_LIMIT,
        offset,
    })
}

/// Query the daemon and rebuild the grid, empty state or error state.
pub fn refresh(ui: &Rc<Ui>) {
    // Record the revision we are about to render so the live-refresh poller
    // does not rebuild the grid a second time right after this.
    sync_status(ui);
    let previous = current_child(ui).map(|c| c.index());
    let grid_focused = ui.flow.focus_child().is_some();
    while let Some(child) = ui.flow.first_child() {
        ui.flow.remove(&child);
    }
    ui.entries.borrow_mut().clear();
    ui.exhausted.set(true);
    ui.load_more.set_visible(false);

    let query = ui.search.text().trim().to_string();
    let filter = ui.filter.get();
    let entries = match call(&current_query(ui, 0)) {
        Ok(ResponseData::Entries(entries)) => entries,
        Ok(_) => Vec::new(),
        Err(panora_core::Error::Ipc(message)) if !message.starts_with("daemon unavailable") => {
            ui.error.set_title(ui.s.error_daemon_title);
            // StatusPage descriptions are parsed as Pango markup, so anything
            // that did not come from this file has to be escaped first.
            ui.error
                .set_description(Some(&glib::markup_escape_text(&message)));
            ui.stack.set_visible_child_name("error");
            ui.title.set_subtitle(ui.s.subtitle_connection_problem);
            return;
        }
        Err(error) => {
            ui.error.set_title(ui.s.error_connect_title);
            ui.error.set_description(Some(&format!(
                "{}\n\n{}",
                glib::markup_escape_text(ui.s.error_connect_description),
                glib::markup_escape_text(&error.to_string())
            )));
            ui.stack.set_visible_child_name("error");
            ui.title.set_subtitle(ui.s.subtitle_no_connection);
            return;
        }
    };

    if entries.is_empty() {
        ui.title.set_subtitle(&subtitle_for(ui.s, 0, false, filter));
        if query.is_empty() {
            ui.empty.set_icon_name(Some("edit-paste-symbolic"));
            ui.empty.set_title(if filter == Filter::All {
                ui.s.empty_title
            } else {
                ui.s.empty_filter_title
            });
            ui.empty.set_description(Some(ui.s.empty_description));
        } else {
            ui.empty.set_icon_name(Some("system-search-symbolic"));
            ui.empty.set_title(ui.s.no_results_title);
            // The search term is user input landing in a markup-parsed label.
            ui.empty.set_description(Some(&fill(
                ui.s.no_results_description,
                "q",
                &glib::markup_escape_text(&query),
            )));
        }
        ui.stack.set_visible_child_name("empty");
        return;
    }

    append_entries(ui, entries);
    update_subtitle(ui);
    ui.stack.set_visible_child_name("list");

    // Keep the user's place after pin/delete: reselect the same position
    // (or the new last card) and restore keyboard focus to the grid.
    if let Some(index) = previous {
        let count = ui.entries.borrow().len() as i32;
        let index = index.min(count.saturating_sub(1));
        if let Some(child) = ui.flow.child_at_index(index) {
            if grid_focused {
                child.grab_focus();
            }
            ui.flow.select_child(&child);
        }
    }
}

fn update_subtitle(ui: &Rc<Ui>) {
    ui.title.set_subtitle(&subtitle_for(
        ui.s,
        ui.entries.borrow().len(),
        !ui.exhausted.get(),
        ui.filter.get(),
    ));
}

/// Fetch the next page and append it to the grid.
fn load_next_page(ui: &Rc<Ui>) {
    if ui.exhausted.get() {
        return;
    }
    let offset = ui.entries.borrow().len();
    match call(&current_query(ui, offset)) {
        Ok(ResponseData::Entries(entries)) if !entries.is_empty() => {
            append_entries(ui, entries);
            update_subtitle(ui);
        }
        _ => {
            ui.exhausted.set(true);
            ui.load_more.set_visible(false);
            update_subtitle(ui);
        }
    }
}

fn append_entries(ui: &Rc<Ui>, entries: Vec<Entry>) {
    let more = entries.len() >= PAGE_LIMIT;
    let offset = ui.entries.borrow().len();
    for (i, entry) in entries.iter().enumerate() {
        let child = build_card(ui, entry, offset + i);
        ui.flow.insert(&child, -1);
    }
    ui.entries.borrow_mut().extend(entries);
    ui.exhausted.set(!more);
    ui.load_more.set_visible(more);
}

/// Human-readable record count for the header subtitle; `more` marks a
/// partially loaded list ("60+").
fn subtitle_for(s: &Strings, count: usize, more: bool, filter: Filter) -> String {
    let shown = if more {
        format!("{count}+")
    } else {
        count.to_string()
    };
    let base = fill(s.subtitle_count, "n", &shown);
    match filter {
        Filter::All => base,
        other => format!("{base} · {}", other.label(s).to_lowercase()),
    }
}

/// Give an icon-only control a name assistive technology can announce.
///
/// A tooltip is not one: AT-SPI exposes it as the *description*, which a
/// screen reader reads after the name -- and an unnamed button has no name to
/// read, so it is announced as just "button". Every icon-only control here
/// therefore carries both.
fn name_for_a11y(widget: &impl IsA<gtk::Widget>, label: &str) {
    widget
        .as_ref()
        .update_property(&[gtk::accessible::Property::Label(label)]);
}

/// One row of the history: the content first, then a quiet metadata line
/// whose actions surface on hover, focus or selection.
///
/// Actions stay in the layout at `opacity: 0` rather than being added and
/// removed, so nothing shifts under the pointer when a row lights up, and a
/// keyboard user can still Tab into them (`:focus-within` reveals them).
fn build_card(ui: &Rc<Ui>, entry: &Entry, index: usize) -> gtk::FlowBoxChild {
    let s = ui.s;
    let card = gtk::Box::new(gtk::Orientation::Vertical, 8);
    card.add_css_class("history-card");

    match entry.kind {
        ContentKind::Image => {
            let picture = gtk::Picture::new();
            picture.set_size_request(-1, 132);
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
        _ => card.append(&preview_label(&entry.preview, 3)),
    }

    let bottom = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    bottom.set_valign(gtk::Align::End);

    // The first rows carry their Ctrl+N number, quietly, at the start of
    // the metadata line.
    if index < QUICK_PICK_ROWS {
        let number = gtk::Label::new(Some(&(index + 1).to_string()));
        number.add_css_class("card-index");
        number.set_tooltip_text(Some(&format!("Ctrl+{}", index + 1)));
        bottom.append(&number);
    }

    // The kind is an icon here, not a shouted badge: the preview already
    // shows what the entry is, and the badge only repeated it in 9px caps.
    // Screen readers get the kind from the row's accessible label instead.
    let icon = gtk::Image::from_icon_name(kind_icon(entry.kind));
    icon.add_css_class("kind-icon");
    icon.set_pixel_size(14);
    bottom.append(&icon);

    let meta = gtk::Label::new(Some(&card_meta(s, entry)));
    meta.add_css_class("card-meta");
    meta.set_halign(gtk::Align::Start);
    meta.set_ellipsize(gtk::pango::EllipsizeMode::End);
    meta.set_hexpand(true);
    bottom.append(&meta);

    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    actions.add_css_class("card-actions");
    // A pinned row must read as pinned without being hovered first.
    if entry.pinned {
        actions.add_css_class("always-visible");
    }

    let pin = gtk::Button::from_icon_name(if entry.pinned {
        "starred-symbolic"
    } else {
        "non-starred-symbolic"
    });
    let pin_label = if entry.pinned {
        s.tooltip_unpin
    } else {
        s.tooltip_pin
    };
    pin.add_css_class("flat");
    pin.add_css_class("card-action");
    pin.set_tooltip_text(Some(pin_label));
    name_for_a11y(&pin, pin_label);
    if entry.pinned {
        pin.add_css_class("pinned");
    }
    {
        let ui = ui.clone();
        let id = entry.id;
        let pinned = entry.pinned;
        pin.connect_clicked(move |_| toggle_pin(&ui, id, pinned));
    }
    actions.append(&pin);

    let info = gtk::Button::from_icon_name("view-more-symbolic");
    info.add_css_class("flat");
    info.add_css_class("card-action");
    info.set_tooltip_text(Some(s.tooltip_details));
    name_for_a11y(&info, s.tooltip_details);
    {
        let ui = ui.clone();
        let entry = entry.clone();
        info.connect_clicked(move |_| details::show(&ui, &entry));
    }
    actions.append(&info);

    let delete = gtk::Button::from_icon_name("user-trash-symbolic");
    delete.add_css_class("flat");
    delete.add_css_class("card-action");
    delete.set_tooltip_text(Some(s.tooltip_delete));
    name_for_a11y(&delete, s.tooltip_delete);
    {
        let ui = ui.clone();
        let id = entry.id;
        delete.connect_clicked(move |_| delete_entry(&ui, id));
    }
    actions.append(&delete);
    bottom.append(&actions);
    card.append(&bottom);

    let child = gtk::FlowBoxChild::new();
    child.set_child(Some(&card));
    child.set_tooltip_text(Some(&format!(
        "{}\n{}",
        entry.primary_mime, s.tooltip_click_to_copy
    )));
    // Kind, then content, then age: what a sighted user takes in from the
    // row at a glance, in the order they take it in.
    name_for_a11y(
        &child,
        &format!(
            "{}: {} — {}",
            kind_label(s, entry.kind),
            entry.preview.trim(),
            relative_time(s, entry.last_seen_at)
        ),
    );
    child
}

/// Quiet metadata line under the content: age, size, and the source
/// application when the backend could name it.
fn card_meta(s: &Strings, entry: &Entry) -> String {
    let mut parts = vec![
        relative_time(s, entry.last_seen_at),
        format_size(entry.size_bytes),
    ];
    if let Some(app) = entry.source_app.as_deref().filter(|app| !app.is_empty()) {
        parts.push(app.to_string());
    }
    parts.join(" · ")
}

/// Draw the parsed color instead of a fixed placeholder swatch.
pub fn color_swatch(preview: &str) -> gtk::DrawingArea {
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
/// The window is hidden first so focus (and the optional paste) lands in
/// the application the user came from.
pub fn recall(ui: &Rc<Ui>, id: i64) {
    recall_with(ui, id, None);
}

/// `recall` restricted to one format when `mime` is given (Shift+Enter puts
/// only the plain text of a rich-text entry on the clipboard).
pub fn recall_with(ui: &Rc<Ui>, id: i64, mime: Option<&'static str>) {
    let paste = ui.app.config.borrow().ui.instant_paste;
    ui.window.set_visible(false);
    // Let the main loop process the unmap first, then talk to the daemon on
    // a worker thread: focus must be back in the target application before
    // panod sends Ctrl+V, and a blocking call here would delay the unmap.
    let ui = ui.clone();
    glib::timeout_add_local_once(Duration::from_millis(60), move || {
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(call(&Request::Recall {
                id,
                paste,
                mime: mime.map(String::from),
            }));
        });
        glib::timeout_add_local(Duration::from_millis(20), move || {
            let result = match receiver.try_recv() {
                Ok(result) => result,
                Err(TryRecvError::Empty) => return glib::ControlFlow::Continue,
                Err(TryRecvError::Disconnected) => {
                    ui.window.close();
                    return glib::ControlFlow::Break;
                }
            };
            match result {
                Ok(ResponseData::Recalled { pasted }) => {
                    if paste && !pasted {
                        notify(&ui, ui.s.toast_paste_failed);
                    }
                    ui.window.close();
                }
                Ok(_) => ui.window.close(),
                Err(_) => {
                    ui.window.set_visible(true);
                    toast(&ui, ui.s.toast_recall_failed);
                    // Hiding the window stopped the poller; bring it back.
                    start_live_refresh(&ui);
                }
            }
            glib::ControlFlow::Break
        });
    });
}

/// Put only one format of an entry on the clipboard (plain text of a rich
/// text entry, for instance). Routed through the daemon so ownership does
/// not die with this process.
pub fn recall_as(ui: &Rc<Ui>, id: i64, mime: &str) -> bool {
    match call(&Request::Recall {
        id,
        paste: false,
        mime: Some(mime.to_string()),
    }) {
        Ok(_) => true,
        Err(_) => {
            toast(ui, ui.s.toast_recall_failed);
            false
        }
    }
}

/// Desktop notification for outcomes the (already hidden) popup cannot show.
fn notify(ui: &Rc<Ui>, message: &str) {
    if let Some(app) = ui.window.application() {
        let notification = gtk::gio::Notification::new(ui.s.app_name);
        notification.set_body(Some(message));
        app.send_notification(Some("panora-paste"), &notification);
    }
}

pub fn toggle_pin(ui: &Rc<Ui>, id: i64, pinned: bool) {
    match call(&Request::Pin {
        id,
        pinned: !pinned,
    }) {
        Ok(_) => {
            toast(
                ui,
                if pinned {
                    ui.s.toast_unpinned
                } else {
                    ui.s.toast_pinned
                },
            );
            refresh(ui);
        }
        Err(_) => toast(ui, ui.s.toast_pin_failed),
    }
}

pub fn delete_entry(ui: &Rc<Ui>, id: i64) {
    match call(&Request::Delete { id }) {
        Ok(_) => {
            // The daemon keeps the tombstone and blobs for a grace period,
            // so the toast can offer a real undo.
            let undo_toast = adw::Toast::builder()
                .title(ui.s.toast_deleted)
                .button_label(ui.s.toast_undo)
                .timeout(5)
                .build();
            let undo = ui.clone();
            undo_toast.connect_button_clicked(move |_| match call(&Request::Restore { id }) {
                Ok(_) => {
                    toast(&undo, undo.s.toast_restored);
                    refresh(&undo);
                }
                Err(_) => toast(&undo, undo.s.toast_restore_failed),
            });
            ui.toasts.add_toast(undo_toast);
            refresh(ui);
        }
        Err(_) => toast(ui, ui.s.toast_delete_failed),
    }
}

pub fn toast(ui: &Rc<Ui>, message: &str) {
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
/// composite widget and the focus actually lands on its inner GtkText.
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

/// Fetch an image payload off the GTK thread and hand a texture to `picture`.
pub fn load_image_preview_async(id: i64, picture: gtk::Picture) {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let result = call(&Request::Preview {
            id,
            thumbnail: true,
        })
        .ok();
        let _ = sender.send(result);
    });

    glib::timeout_add_local(Duration::from_millis(20), move || {
        let response = match receiver.try_recv() {
            Ok(response) => response,
            Err(TryRecvError::Empty) => return glib::ControlFlow::Continue,
            Err(TryRecvError::Disconnected) => return glib::ControlFlow::Break,
        };
        let Some(ResponseData::Payloads(payloads)) = response else {
            return glib::ControlFlow::Break;
        };
        let Some(payload) = payloads.into_iter().find(|p| p.mime.starts_with("image/")) else {
            return glib::ControlFlow::Break;
        };
        if let Some(texture) = texture_from_bytes(&payload.data, 320, 180) {
            picture.set_paintable(Some(&texture));
        }
        glib::ControlFlow::Break
    });
}

/// Decode image bytes into a texture, scaled down to the given box.
pub fn texture_from_bytes(bytes: &[u8], width: i32, height: i32) -> Option<gdk::Texture> {
    let loader = PixbufLoader::new();
    // Scale to fit the box while keeping the aspect ratio; `set_size` alone
    // would stretch the image to exactly width x height.
    loader.connect_size_prepared(move |loader, w, h| {
        if w <= 0 || h <= 0 {
            return;
        }
        let scale = (f64::from(width) / f64::from(w))
            .min(f64::from(height) / f64::from(h))
            .min(1.0);
        loader.set_size(
            ((f64::from(w) * scale).round() as i32).max(1),
            ((f64::from(h) * scale).round() as i32).max(1),
        );
    });
    if loader.write(bytes).is_err() || loader.close().is_err() {
        return None;
    }
    let pixbuf = loader.pixbuf()?;
    Some(gdk::Texture::for_pixbuf(&pixbuf))
}

pub fn install_css() {
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

         flowboxchild { padding: 0; border-radius: 12px; }
         flowboxchild:selected { background: none; }
         /* Keyboard focus must be visible on its own, not only through the
            selection tint: WCAG 2.4.7, and the grid can hold the cursor
            without a selection. */
         flowboxchild:focus-visible .history-card {
             outline: 2px solid @accent_color;
             outline-offset: -2px;
         }

         .history-card {
             background: @card_bg_color;
             border: 1px solid alpha(@card_fg_color, 0.11);
             border-radius: 12px;
             padding: 10px 12px;
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

         /* 0.7 alpha keeps these above the 3:1 non-text contrast floor
            (WCAG 1.4.11) in both light and dark. */
         .kind-icon { color: alpha(@card_fg_color, 0.7); }
         .card-index {
             font-size: 0.75em;
             font-weight: 700;
             font-feature-settings: \"tnum\";
             min-width: 14px;
             color: alpha(@card_fg_color, 0.55);
         }

         .preview-text { color: @card_fg_color; }
         /* No px font-size: .caption-heading tracks the user's text scale. */
         .card-meta {
             font-size: 0.8em;
             color: alpha(@card_fg_color, 0.7);
         }

         /* Revealed by the row, so a resting list is content and nothing
            else. Opacity rather than visibility: the buttons keep their
            space (no shift under the pointer) and stay reachable by Tab.
            The row is reached three ways and all three must reveal it --
            pointer, keyboard cursor and keyboard selection. GtkFlowBox puts
            focus and selection on the flowboxchild *around* the card, so
            `.history-card:focus-within` alone would never match while
            arrowing down the list. */
         .card-actions { opacity: 0; transition: opacity 120ms ease; }
         .history-card:hover .card-actions,
         flowboxchild:selected .card-actions,
         flowboxchild:focus-within .card-actions,
         .card-actions.always-visible { opacity: 1; }

         /* 28px: WCAG 2.2 target size (2.5.8) asks for at least 24x24, and
            these sit 2px apart, so the extra keeps neighbours apart too. */
         .card-action {
             min-width: 28px;
             min-height: 28px;
             padding: 2px;
             opacity: 0.8;
         }
         .card-action:hover { opacity: 1; }
         .card-action.pinned { color: @accent_color; opacity: 1; }

         .image-preview { border-radius: 8px; background: alpha(@card_fg_color, 0.06); }
         .color-swatch { border-radius: 8px; }
         .details-image { border-radius: 12px; }

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
