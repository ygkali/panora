// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! The Devices page of the preferences: the sync service, this device, the
//! devices of its group, and pairing (ROADMAP SYNC-04).
//!
//! Everything goes through the control socket of the separate
//! `panora-sync` service ([`panora_core::sync::control`]); this binary
//! links no network code. Without the service the page says so, and when
//! the package is installed it offers to start it.
//!
//! Pairing runs in a subpage. Its request stays open while the subpage is
//! shown; leaving the subpage or closing the dialog closes the connection,
//! which the service takes as "cancel".

use crate::util::{qr_picture, spawn};
use gtk::gdk;
use gtk4 as gtk;
use libadwaita as adw;
use libadwaita::prelude::*;
use panora_core::i18n::{fill, Strings};
use panora_core::sync::control::{
    socket_path, Client, Event, Failure, Handle, Outcome, Request, Status,
};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

/// The service's user unit.
const UNIT: &str = "panora-sync.service";
/// How often the page asks the service which devices are connected.
const REFRESH_SECS: u32 = 5;
/// How long a status, remove or leave request may take.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// Polls (half a second apart) for a service that was just started or
/// stopped to get there.
const SERVICE_POLLS: u32 = 20;

/// What the page found.
enum Service {
    /// Answering; its report.
    Running(Status),
    /// Installed, not running.
    Stopped,
    /// Not installed.
    Missing,
}

/// The page and what it shows.
struct Devices {
    s: &'static Strings,
    dialog: adw::PreferencesDialog,
    service_row: adw::SwitchRow,
    missing_row: adw::ActionRow,
    this_group: adw::PreferencesGroup,
    this_row: adw::ActionRow,
    notice: gtk::Label,
    devices_group: adw::PreferencesGroup,
    device_rows: RefCell<Vec<gtk::Widget>>,
    add_group: adw::PreferencesGroup,
    join_group: adw::PreferencesGroup,
    leave_group: adw::PreferencesGroup,
    /// The report the rows were last built from.
    shown: RefCell<Option<Status>>,
    probing: Cell<bool>,
    /// A refresh was asked for while one ran: its answer may be stale.
    again: Cell<bool>,
    /// Whether the unit is installed, once asked: while the service is
    /// down the page need not run systemctl every few seconds.
    installed: Cell<Option<bool>>,
    /// The switch is being set from the service's state, not by the user.
    reverting: Cell<bool>,
    /// The user flipped the switch and the service has not got there yet.
    toggling: Cell<bool>,
    /// False once the dialog closed: timers stop, answers are dropped.
    open: Cell<bool>,
    /// The pairing subpage on screen, if any.
    flow: RefCell<Option<Rc<Flow>>>,
}

/// Build the Devices page for `dialog`.
pub fn page(s: &'static Strings, dialog: &adw::PreferencesDialog) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::builder()
        .title(s.sync_page)
        .icon_name("emblem-synchronizing-symbolic")
        .build();

    let service_group = adw::PreferencesGroup::builder()
        .title(s.sync_title)
        .description(s.sync_title_sub)
        .build();
    let service_row = adw::SwitchRow::builder()
        .title(s.sync_service)
        .subtitle(s.sync_service_sub)
        .sensitive(false)
        .build();
    let missing_row = adw::ActionRow::builder()
        .title(s.sync_missing)
        .subtitle(s.sync_missing_sub)
        .visible(false)
        .build();
    missing_row.add_prefix(&gtk::Image::from_icon_name("dialog-information-symbolic"));
    service_group.add(&service_row);
    service_group.add(&missing_row);
    page.add(&service_group);

    let this_group = adw::PreferencesGroup::builder()
        .title(s.sync_this_device)
        .visible(false)
        .build();
    // Device names come from other devices too; rows show them as text.
    let this_row = adw::ActionRow::builder()
        .use_markup(false)
        .title_lines(1)
        .build();
    this_row.add_prefix(&gtk::Image::from_icon_name("computer-symbolic"));
    this_group.add(&this_row);
    let notice = gtk::Label::new(None);
    notice.add_css_class("caption");
    notice.add_css_class("warning");
    notice.set_wrap(true);
    notice.set_xalign(0.0);
    notice.set_margin_top(6);
    notice.set_visible(false);
    this_group.add(&notice);
    page.add(&this_group);

    let devices_group = adw::PreferencesGroup::builder()
        .title(s.sync_devices)
        .visible(false)
        .build();
    page.add(&devices_group);

    let add_group = adw::PreferencesGroup::builder()
        .title(s.sync_add)
        .visible(false)
        .build();
    let pair_row = nav_row(s.sync_pair_code, s.sync_pair_code_sub);
    let invite_row = nav_row(s.sync_invite_link, s.sync_invite_link_sub);
    add_group.add(&pair_row);
    add_group.add(&invite_row);
    page.add(&add_group);

    let join_group = adw::PreferencesGroup::builder()
        .title(s.sync_join)
        .visible(false)
        .build();
    let join_code_row = nav_row(s.sync_join_code, s.sync_join_code_sub);
    let join_link_row = nav_row(s.sync_join_link, s.sync_join_link_sub);
    join_group.add(&join_code_row);
    join_group.add(&join_link_row);
    page.add(&join_group);

    let leave_group = adw::PreferencesGroup::builder().visible(false).build();
    let leave = gtk::Button::with_label(s.sync_leave);
    leave.add_css_class("destructive-action");
    leave.add_css_class("pill");
    leave.set_halign(gtk::Align::Center);
    leave_group.add(&leave);
    page.add(&leave_group);

    let devices = Rc::new(Devices {
        s,
        dialog: dialog.clone(),
        service_row,
        missing_row,
        this_group,
        this_row,
        notice,
        devices_group,
        device_rows: RefCell::new(Vec::new()),
        add_group,
        join_group,
        leave_group,
        shown: RefCell::new(None),
        probing: Cell::new(false),
        again: Cell::new(false),
        installed: Cell::new(None),
        reverting: Cell::new(false),
        toggling: Cell::new(false),
        open: Cell::new(true),
        flow: RefCell::new(None),
    });

    {
        let row = devices.service_row.clone();
        let devices = devices.clone();
        row.connect_active_notify(move |row| devices.service_switched(row.is_active()));
    }
    {
        let devices = devices.clone();
        pair_row.connect_activated(move |_| pair(&devices));
    }
    {
        let devices = devices.clone();
        invite_row.connect_activated(move |_| invite(&devices));
    }
    {
        let devices = devices.clone();
        join_code_row.connect_activated(move |_| join_code(&devices));
    }
    {
        let devices = devices.clone();
        join_link_row.connect_activated(move |_| join_link(&devices));
    }
    {
        let devices = devices.clone();
        leave.connect_clicked(move |_| {
            let s = devices.s;
            let target = devices.clone();
            devices.confirm(
                s.sync_leave_title,
                s.sync_leave_body,
                s.sync_leave,
                move || target.request_once(Request::Leave),
            );
        });
    }
    {
        let devices = devices.clone();
        dialog.connect_closed(move |_| {
            devices.open.set(false);
            let flow = devices.flow.borrow_mut().take();
            if let Some(flow) = flow {
                flow.cancel();
            }
        });
    }
    {
        let devices = devices.clone();
        glib::timeout_add_seconds_local(REFRESH_SECS, move || {
            if !devices.open.get() {
                return glib::ControlFlow::Break;
            }
            devices.refresh();
            glib::ControlFlow::Continue
        });
    }
    devices.refresh();
    page
}

/// An activatable row that opens a subpage.
fn nav_row(title: &str, subtitle: &str) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(title)
        .subtitle(subtitle)
        .activatable(true)
        .build();
    row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
    row
}

impl Devices {
    /// Ask the service (on a worker thread) and show what it says.
    fn refresh(self: &Rc<Self>) {
        if self.probing.replace(true) {
            self.again.set(true);
            return;
        }
        let this = self.clone();
        let installed = self.installed.get();
        spawn(
            move || probe(installed),
            move |service| {
                this.probing.set(false);
                if this.again.replace(false) {
                    // Asked before a remove or a pairing finished; ask again.
                    this.refresh();
                } else if this.open.get() {
                    this.show(service);
                }
            },
        );
    }

    /// Rebuild what changed since the last report.
    fn show(self: &Rc<Self>, service: Service) {
        let missing = matches!(service, Service::Missing);
        if !matches!(service, Service::Running(_)) {
            self.installed.set(Some(!missing));
        }
        self.missing_row.set_visible(missing);
        self.service_row.set_visible(!missing);
        let status = match service {
            Service::Running(status) => Some(status),
            Service::Stopped | Service::Missing => None,
        };
        if !self.toggling.get() {
            self.set_switch(status.is_some());
            self.service_row.set_sensitive(!missing);
        }
        let Some(status) = status else {
            for group in [
                &self.this_group,
                &self.devices_group,
                &self.add_group,
                &self.join_group,
                &self.leave_group,
            ] {
                group.set_visible(false);
            }
            self.shown.replace(None);
            return;
        };
        if self.shown.borrow().as_ref() == Some(&status) {
            return;
        }
        let s = self.s;
        self.this_group.set_visible(true);
        self.this_row.set_title(&status.device_name);
        self.this_row
            .set_subtitle(&fill(s.sync_fingerprint, "f", &status.fingerprint));
        let notice = if status.in_group && !status.member {
            Some(s.sync_notice_removed)
        } else if status.member && !status.has_key {
            Some(s.sync_notice_key)
        } else {
            None
        };
        self.notice.set_label(notice.unwrap_or(""));
        self.notice.set_visible(notice.is_some());

        self.devices_group
            .set_visible(status.in_group && status.member);
        for row in self.device_rows.borrow_mut().drain(..) {
            self.devices_group.remove(&row);
        }
        let others: Vec<_> = status.devices.iter().filter(|d| !d.this_device).collect();
        let mut rows = Vec::new();
        if others.is_empty() {
            let row = adw::ActionRow::builder()
                .title(s.sync_no_devices)
                .subtitle(s.sync_no_devices_sub)
                .build();
            rows.push(row.upcast::<gtk::Widget>());
        }
        for device in others {
            let row = adw::ActionRow::builder()
                .title(&device.name)
                .subtitle(fill(s.sync_fingerprint, "f", &device.fingerprint))
                .use_markup(false)
                .title_lines(1)
                .subtitle_lines(1)
                .build();
            row.add_prefix(&gtk::Image::from_icon_name("computer-symbolic"));
            let state = gtk::Label::new(Some(if device.connected {
                s.sync_connected
            } else {
                s.sync_not_connected
            }));
            state.add_css_class("caption");
            if device.connected {
                state.add_css_class("success");
            } else {
                state.add_css_class("dim-label");
            }
            row.add_suffix(&state);
            let remove = gtk::Button::from_icon_name("user-trash-symbolic");
            remove.add_css_class("flat");
            remove.set_valign(gtk::Align::Center);
            remove.set_tooltip_text(Some(s.sync_remove));
            remove.update_property(&[gtk::accessible::Property::Label(&fill(
                s.sync_remove_named,
                "name",
                &device.name,
            ))]);
            {
                let this = Rc::downgrade(self);
                let name = device.name.clone();
                let fingerprint = device.fingerprint.clone();
                remove.connect_clicked(move |_| {
                    let Some(this) = this.upgrade() else {
                        return;
                    };
                    let target = this.clone();
                    let fingerprint = fingerprint.clone();
                    this.confirm(
                        &fill(s.sync_remove_title, "name", &name),
                        s.sync_remove_body,
                        s.sync_remove,
                        move || {
                            target.request_once(Request::Remove {
                                device: fingerprint.clone(),
                            })
                        },
                    );
                });
            }
            row.add_suffix(&remove);
            rows.push(row.upcast::<gtk::Widget>());
        }
        for row in &rows {
            self.devices_group.add(row);
        }
        *self.device_rows.borrow_mut() = rows;

        self.add_group.set_visible(status.can_invite());
        self.join_group.set_visible(status.can_join());
        self.leave_group.set_visible(status.in_group);
        self.shown.replace(Some(status));
    }

    fn set_switch(&self, active: bool) {
        self.reverting.set(true);
        self.service_row.set_active(active);
        self.reverting.set(false);
    }

    fn toast(&self, text: &str) {
        self.dialog.add_toast(
            adw::Toast::builder()
                .title(text)
                .use_markup(false)
                .timeout(3)
                .build(),
        );
    }

    /// The user flipped the service switch: enable and start the unit, or
    /// stop and disable it, then wait for the service to get there.
    fn service_switched(self: &Rc<Self>, on: bool) {
        if self.reverting.get() {
            return;
        }
        self.toggling.set(true);
        self.service_row.set_sensitive(false);
        let this = self.clone();
        spawn(
            move || set_unit(on),
            move |result| match result {
                Ok(()) => this.await_service(on, SERVICE_POLLS),
                Err(e) => {
                    this.toast(&fill(this.s.sync_service_failed, "e", &e));
                    this.toggling.set(false);
                    this.service_row.set_sensitive(true);
                    this.set_switch(!on);
                }
            },
        );
    }

    fn await_service(self: &Rc<Self>, on: bool, polls_left: u32) {
        let this = self.clone();
        spawn(
            || probe(None),
            move |service| {
                if !this.open.get() {
                    return;
                }
                let arrived = matches!(service, Service::Running(_)) == on;
                if arrived || polls_left == 0 {
                    this.toggling.set(false);
                    if !arrived {
                        this.toast(this.s.sync_service_slow);
                    }
                    this.show(service);
                    return;
                }
                glib::timeout_add_local_once(Duration::from_millis(500), move || {
                    this.await_service(on, polls_left - 1)
                });
            },
        );
    }

    /// Ask before something that cannot be undone from here.
    fn confirm(&self, heading: &str, body: &str, action: &str, then: impl Fn() + 'static) {
        let alert = adw::AlertDialog::new(Some(heading), Some(body));
        alert.add_response("cancel", self.s.sync_cancel);
        alert.add_response("confirm", action);
        alert.set_response_appearance("confirm", adw::ResponseAppearance::Destructive);
        alert.set_default_response(Some("cancel"));
        alert.set_close_response("cancel");
        alert.connect_response(None, move |_, response| {
            if response == "confirm" {
                then();
            }
        });
        alert.present(Some(&self.dialog));
    }

    /// Remove or leave: one request, one answer, then a fresh report.
    fn request_once(self: &Rc<Self>, request: Request) {
        let this = self.clone();
        spawn(
            move || request_once(request),
            move |result| {
                let s = this.s;
                match result {
                    Ok(Outcome::Removed { name, .. }) => {
                        this.toast(&fill(s.sync_removed, "name", &name));
                    }
                    Ok(Outcome::Left) => this.toast(s.sync_left),
                    Ok(_) => {}
                    Err((failure, message)) => this.toast(&failure.describe(s, &message)),
                }
                this.shown.replace(None);
                this.refresh();
            },
        );
    }
}

/// One pairing request, in a subpage of the dialog.
struct Flow {
    devices: Rc<Devices>,
    status: adw::StatusPage,
    /// Attempts that failed while the window stays open.
    note: gtk::Label,
    handle: RefCell<Option<Handle>>,
    /// The request went out; a second click does not send another.
    started: Cell<bool>,
    /// Finished, failed or cancelled: later events are dropped.
    over: Cell<bool>,
    /// When the code window closes (pairing with a code), to go back to
    /// waiting after a failed attempt.
    listening_until: Cell<Option<i64>>,
}

/// What the reader thread hands to the main loop.
enum Msg {
    /// Connected and the request sent; the handle answers and cancels.
    Connected(Handle),
    /// The next event, the end of the stream, or a read error.
    Next(Result<Option<Event>, String>),
}

impl Flow {
    /// Push a subpage titled `title`.
    fn open(devices: &Rc<Devices>, title: &str) -> Rc<Flow> {
        let status = adw::StatusPage::new();
        let note = gtk::Label::new(None);
        note.add_css_class("caption");
        note.add_css_class("warning");
        note.set_wrap(true);
        note.set_margin_start(12);
        note.set_margin_end(12);
        note.set_margin_top(6);
        note.set_visible(false);
        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&adw::HeaderBar::new());
        // Under the header rather than at the bottom, where a bottom sheet
        // on a short screen would cut it off.
        toolbar.add_top_bar(&note);
        toolbar.set_content(Some(&status));
        let nav = adw::NavigationPage::builder()
            .title(title)
            .child(&toolbar)
            .build();
        let flow = Rc::new(Flow {
            devices: devices.clone(),
            status,
            note,
            handle: RefCell::new(None),
            started: Cell::new(false),
            over: Cell::new(false),
            listening_until: Cell::new(None),
        });
        {
            // Back, Escape or a swipe: whatever is in progress stops.
            let flow = Rc::downgrade(&flow);
            nav.connect_hidden(move |_| {
                if let Some(flow) = flow.upgrade() {
                    flow.cancel();
                }
            });
        }
        devices.flow.replace(Some(flow.clone()));
        devices.dialog.push_subpage(&nav);
        flow
    }

    /// Close the connection; the service cancels the request.
    fn cancel(&self) {
        self.over.set(true);
        if let Some(handle) = self.handle.borrow_mut().take() {
            handle.cancel();
        }
        // Only this flow's own slot: a late signal from an older page must
        // not forget the one on screen.
        let mine = {
            let mut slot = self.devices.flow.borrow_mut();
            if slot
                .as_ref()
                .is_some_and(|flow| std::ptr::eq(Rc::as_ptr(flow), self))
            {
                slot.take()
            } else {
                None
            }
        };
        drop(mine);
    }

    fn set(&self, icon: Option<&str>, title: &str, description: &str, child: Option<&gtk::Widget>) {
        self.status.set_icon_name(icon);
        self.status.set_title(title);
        // The description is Pango markup; everything shown here is text,
        // including device names that other devices chose.
        self.status
            .set_description(Some(glib::markup_escape_text(description).as_str()));
        self.status.set_child(child);
    }

    /// Something is in progress.
    fn busy(&self, title: &str, description: &str) {
        let spinner = gtk::Spinner::builder()
            .spinning(true)
            .width_request(32)
            .height_request(32)
            .halign(gtk::Align::Center)
            .build();
        self.set(None, title, description, Some(spinner.upcast_ref()));
    }

    fn note(&self, text: &str) {
        self.note.set_label(text);
        self.note.set_visible(true);
    }

    /// Send `request` and hand every event that is not the end of it to
    /// `handler`, on the GTK main loop. Connecting, sending and reading
    /// happen on a worker thread, so a stuck service cannot freeze the
    /// dialog.
    fn start(self: &Rc<Self>, request: Request, handler: impl Fn(&Rc<Flow>, Event) + 'static) {
        if self.over.get() || self.started.replace(true) {
            return;
        }
        let (tx, rx) = async_channel::unbounded();
        std::thread::spawn(move || {
            let connected = Client::connect(&socket_path()).and_then(|mut client| {
                client.send(&request)?;
                let handle = client.handle()?;
                Ok((client, handle))
            });
            let mut client = match connected {
                Ok((client, handle)) => {
                    if tx.send_blocking(Msg::Connected(handle)).is_err() {
                        return;
                    }
                    client
                }
                Err(e) => {
                    let _ = tx.send_blocking(Msg::Next(Err(e.to_string())));
                    return;
                }
            };
            loop {
                let next = client.next_event().map_err(|e| e.to_string());
                let last = !matches!(&next, Ok(Some(event)) if !event.is_final());
                if tx.send_blocking(Msg::Next(next)).is_err() || last {
                    break;
                }
            }
        });
        let flow = self.clone();
        glib::spawn_future_local(async move {
            while let Ok(msg) = rx.recv().await {
                let next = match msg {
                    Msg::Connected(handle) => {
                        if flow.over.get() {
                            // Cancelled while connecting.
                            handle.cancel();
                            break;
                        }
                        flow.handle.replace(Some(handle));
                        continue;
                    }
                    Msg::Next(next) => next,
                };
                if flow.over.get() {
                    break;
                }
                match next {
                    Ok(Some(Event::Done { outcome })) => flow.done(&outcome),
                    Ok(Some(Event::Error { failure, message })) => flow.failed(failure, &message),
                    Ok(Some(event)) => handler(&flow, event),
                    Ok(None) => flow.failed(Failure::Other, flow.devices.s.sync_service_gone),
                    Err(e) => flow.failed(Failure::Other, &e),
                }
            }
        });
    }

    /// The last screen, with a button that goes back to the device list.
    fn finish(&self, icon: &str, title: &str, description: &str, button: &str) {
        self.over.set(true);
        self.handle.borrow_mut().take();
        self.note.set_visible(false);
        let back = gtk::Button::with_label(button);
        back.add_css_class("pill");
        back.add_css_class("suggested-action");
        back.set_halign(gtk::Align::Center);
        {
            let dialog = self.devices.dialog.clone();
            back.connect_clicked(move |_| {
                dialog.pop_subpage();
            });
        }
        self.set(Some(icon), title, description, Some(back.upcast_ref()));
        back.grab_focus();
        self.devices.shown.replace(None);
        self.devices.refresh();
    }

    fn done(&self, outcome: &Outcome) {
        let s = self.devices.s;
        let (title, body) = match outcome {
            Outcome::DeviceJoined { name, .. } => (
                fill(s.sync_device_joined, "name", name),
                s.sync_device_joined_body,
            ),
            Outcome::JoinedGroup => (s.sync_joined.to_string(), s.sync_joined_body),
            Outcome::Removed { name, .. } => (fill(s.sync_removed, "name", name), ""),
            Outcome::Left => (s.sync_left.to_string(), ""),
        };
        self.finish("emblem-ok-symbolic", &title, body, s.sync_done);
    }

    fn failed(&self, failure: Failure, message: &str) {
        let s = self.devices.s;
        let text = failure.describe(s, message);
        self.finish(
            "dialog-warning-symbolic",
            s.sync_failed,
            &text,
            s.sync_close,
        );
    }

    /// The invitation, as a QR code and a link to copy.
    fn show_link(&self, link: &str, expires_at: i64) {
        let s = self.devices.s;
        let column = gtk::Box::new(gtk::Orientation::Vertical, 12);
        column.set_halign(gtk::Align::Center);
        let copy = gtk::Button::with_label(s.sync_copy_link);
        copy.add_css_class("pill");
        copy.set_halign(gtk::Align::Center);
        {
            let link = link.to_string();
            let dialog = self.devices.dialog.clone();
            copy.connect_clicked(move |button| {
                copy_secret(button, &link);
                dialog.add_toast(
                    adw::Toast::builder()
                        .title(s.sync_link_copied)
                        .timeout(3)
                        .build(),
                );
            });
        }
        // Copying is how two computers pass the link; the code is for a
        // phone's camera. The button comes first so a narrow popup shows
        // it without scrolling.
        column.append(&copy);
        column.append(&waiting_row(s.sync_waiting_device));
        if let Some(picture) = qr_picture(link, 4) {
            picture.update_property(&[gtk::accessible::Property::Label(s.sync_invite_qr)]);
            picture.set_margin_top(6);
            column.append(&picture);
        }
        self.set(
            None,
            s.sync_invite_title,
            &fill(s.sync_invite_body, "time", &clock(expires_at)),
            Some(column.upcast_ref()),
        );
    }

    /// The code this device shows, before the other one has confirmed it.
    fn show_code(&self, code: &str) {
        let s = self.devices.s;
        let column = gtk::Box::new(gtk::Orientation::Vertical, 18);
        column.append(&code_label(code));
        column.append(&waiting_row(s.sync_waiting_confirm));
        self.set(
            None,
            s.sync_code_title,
            s.sync_code_body,
            Some(column.upcast_ref()),
        );
    }

    /// Ask the user to compare the code and admit (or join) the device.
    fn ask(self: &Rc<Self>, code: Option<&str>, name: &str, fingerprint: &str) {
        let s = self.devices.s;
        let who = if name.is_empty() {
            fill(s.sync_compare_device, "f", fingerprint)
        } else {
            // The fingerprint first: it is hex, while a name could itself
            // contain `{f}`.
            fill(&fill(s.sync_compare_named, "f", fingerprint), "name", name)
        };
        let column = gtk::Box::new(gtk::Orientation::Vertical, 18);
        if let Some(code) = code {
            column.append(&code_label(code));
        }
        let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        buttons.set_halign(gtk::Align::Center);
        let differ = gtk::Button::with_label(s.sync_codes_differ);
        differ.add_css_class("pill");
        let accept = gtk::Button::with_label(s.sync_codes_match);
        accept.add_css_class("pill");
        accept.add_css_class("suggested-action");
        buttons.append(&differ);
        buttons.append(&accept);
        column.append(&buttons);
        for (button, answer) in [(&differ, false), (&accept, true)] {
            let flow = self.clone();
            button.connect_clicked(move |_| flow.answer(answer));
        }
        self.set(
            None,
            s.sync_compare_title,
            &format!("{who}\n\n{}", s.sync_compare_body),
            Some(column.upcast_ref()),
        );
        // No default: the user compares first, then picks.
        differ.grab_focus();
    }

    fn answer(&self, accept: bool) {
        let s = self.devices.s;
        let sent = match self.handle.borrow_mut().as_mut() {
            Some(handle) => handle.answer(accept).map_err(|e| e.to_string()),
            None => return,
        };
        match sent {
            Ok(()) => self.busy(s.sync_finishing, ""),
            Err(e) => self.failed(Failure::Other, &e),
        }
    }

    /// A form: `rows` in a boxed list and a button that runs `go`.
    fn form(
        self: &Rc<Self>,
        title: &str,
        description: &str,
        rows: &[&adw::EntryRow],
        button: &str,
        go: impl Fn(&Rc<Flow>) + 'static,
    ) {
        let column = gtk::Box::new(gtk::Orientation::Vertical, 18);
        let list = gtk::ListBox::new();
        list.add_css_class("boxed-list");
        list.set_selection_mode(gtk::SelectionMode::None);
        for row in rows {
            list.append(*row);
        }
        column.append(&list);
        let run = gtk::Button::with_label(button);
        run.add_css_class("pill");
        run.add_css_class("suggested-action");
        run.set_halign(gtk::Align::Center);
        column.append(&run);
        let clamp = adw::Clamp::builder()
            .maximum_size(420)
            .child(&column)
            .build();
        let go = Rc::new(go);
        {
            let flow = self.clone();
            let go = go.clone();
            run.connect_clicked(move |_| go(&flow));
        }
        for row in rows {
            let flow = self.clone();
            let go = go.clone();
            row.connect_entry_activated(move |_| go(&flow));
        }
        self.set(
            Some("emblem-synchronizing-symbolic"),
            title,
            description,
            Some(clamp.upcast_ref()),
        );
        if let Some(first) = rows.first() {
            first.grab_focus();
        }
    }
}

/// A spinner and a line of text, side by side.
fn waiting_row(text: &str) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.set_halign(gtk::Align::Center);
    row.append(&gtk::Spinner::builder().spinning(true).build());
    let label = gtk::Label::new(Some(text));
    label.add_css_class("dim-label");
    row.append(&label);
    row
}

/// The six digits, large.
fn code_label(code: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(code));
    label.add_css_class("panora-pair-code");
    label.set_selectable(false);
    label
}

/// Accept a device by comparing a code (this device is in the group).
fn pair(devices: &Rc<Devices>) {
    let s = devices.s;
    let flow = Flow::open(devices, s.sync_pair_code);
    flow.busy(s.sync_preparing, "");
    let listening = move |flow: &Rc<Flow>, expires_at: i64| {
        flow.listening_until.set(Some(expires_at));
        flow.busy(
            s.sync_listening_title,
            &fill(s.sync_listening_body, "time", &clock(expires_at)),
        );
    };
    flow.start(Request::Pair, move |flow, event| match event {
        Event::Listening { expires_at } => listening(flow, expires_at),
        Event::Code { code } => flow.show_code(&code),
        Event::Ask {
            code,
            name,
            fingerprint,
        } => flow.ask(code.as_deref(), &name, &fingerprint),
        Event::AttemptFailed { failure, message } => {
            flow.note(&fill(
                s.sync_attempt_failed,
                "reason",
                &failure.describe(s, &message),
            ));
            // That code is spent: back to waiting, so the next attempt
            // does not land on an old code or a "finishing" spinner.
            if let Some(expires_at) = flow.listening_until.get() {
                listening(flow, expires_at);
            }
        }
        _ => {}
    });
}

/// Show an invitation link and QR code (this device is in the group).
fn invite(devices: &Rc<Devices>) {
    let s = devices.s;
    let flow = Flow::open(devices, s.sync_invite_link);
    flow.busy(s.sync_preparing, "");
    flow.start(Request::Invite, move |flow, event| match event {
        Event::Link { link, expires_at } => flow.show_link(&link, expires_at),
        Event::AttemptFailed { failure, message } => flow.note(&fill(
            s.sync_attempt_failed,
            "reason",
            &failure.describe(s, &message),
        )),
        _ => {}
    });
}

/// Join the group of a device that waits for a code.
fn join_code(devices: &Rc<Devices>) {
    let s = devices.s;
    let flow = Flow::open(devices, s.sync_join_code);
    let address = adw::EntryRow::builder().title(s.sync_address).build();
    let entry = address.clone();
    flow.form(
        s.sync_join_code,
        s.sync_join_code_body,
        &[&address],
        s.sync_search,
        move |flow| {
            let text = entry.text().trim().to_string();
            let address = (!text.is_empty()).then_some(text);
            flow.busy(s.sync_searching, "");
            flow.start(Request::JoinCode { address }, move |flow, event| {
                if let Event::Ask {
                    code,
                    name,
                    fingerprint,
                } = event
                {
                    flow.ask(code.as_deref(), &name, &fingerprint);
                }
            });
        },
    );
}

/// Join with an invitation link from another device.
fn join_link(devices: &Rc<Devices>) {
    let s = devices.s;
    let flow = Flow::open(devices, s.sync_join_link);
    let link = adw::EntryRow::builder().title(s.sync_link_entry).build();
    let entry = link.clone();
    flow.form(
        s.sync_join_link,
        s.sync_join_link_body,
        &[&link],
        s.sync_join_button,
        move |flow| {
            let link = entry.text().trim().to_string();
            if link.is_empty() {
                return;
            }
            // The link carries a one-time secret; do not leave it around.
            entry.set_text("");
            flow.busy(s.sync_preparing, "");
            flow.start(Request::Join { link }, move |flow, event| {
                if let Event::Joining { fingerprint } = event {
                    flow.busy(&fill(s.sync_joining, "f", &fingerprint), "");
                }
            });
        },
    );
}

/// The service's state, from a worker thread; `installed` is what is
/// already known about the unit.
fn probe(installed: Option<bool>) -> Service {
    if let Some(status) = status_now() {
        return Service::Running(status);
    }
    if installed.unwrap_or_else(unit_installed) {
        Service::Stopped
    } else {
        Service::Missing
    }
}

fn status_now() -> Option<Status> {
    let mut client = Client::connect(&socket_path()).ok()?;
    client.set_timeout(Some(REQUEST_TIMEOUT)).ok()?;
    client.send(&Request::Status).ok()?;
    match client.next_event().ok()?? {
        Event::Status { status } => Some(status),
        _ => None,
    }
}

/// Whether systemd knows the unit (the package is installed).
fn unit_installed() -> bool {
    std::process::Command::new("systemctl")
        .args(["--user", "show", "--property=LoadState", "--value", UNIT])
        .output()
        .map(|output| String::from_utf8_lossy(&output.stdout).trim() == "loaded")
        .unwrap_or(false)
}

/// Enable and start the unit, or stop and disable it.
fn set_unit(on: bool) -> Result<(), String> {
    let verb = if on { "enable" } else { "disable" };
    let output = std::process::Command::new("systemctl")
        .args(["--user", verb, "--now", UNIT])
        .output()
        .map_err(|e| e.to_string())?;
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

/// A request that ends with one answer (remove, leave).
fn request_once(request: Request) -> Result<Outcome, (Failure, String)> {
    let other = |e: std::io::Error| (Failure::Other, e.to_string());
    let mut client = Client::connect(&socket_path()).map_err(other)?;
    client.set_timeout(Some(REQUEST_TIMEOUT)).map_err(other)?;
    client.send(&request).map_err(other)?;
    loop {
        match client.next_event().map_err(other)? {
            Some(Event::Done { outcome }) => return Ok(outcome),
            Some(Event::Error { failure, message }) => return Err((failure, message)),
            Some(_) => {}
            None => {
                return Err((
                    Failure::Other,
                    "the sync service closed the connection".into(),
                ))
            }
        }
    }
}

/// Local wall-clock time (`14:05`) of a Unix timestamp.
fn clock(unix: i64) -> String {
    glib::DateTime::from_unix_local(unix)
        .and_then(|time| time.format("%H:%M"))
        .map(|text| text.to_string())
        .unwrap_or_default()
}

/// Put `text` on the clipboard marked as a secret (the password-manager
/// hint), so Panora's own daemon, and other clipboard managers that honour
/// the hint, keep the one-time link out of their history.
fn copy_secret(widget: &impl IsA<gtk::Widget>, text: &str) {
    let bytes = glib::Bytes::from(text.as_bytes());
    let provider = gdk::ContentProvider::new_union(&[
        gdk::ContentProvider::for_bytes("text/plain;charset=utf-8", &bytes),
        gdk::ContentProvider::for_bytes("text/plain", &bytes),
        gdk::ContentProvider::for_bytes(
            "x-kde-passwordManagerHint",
            &glib::Bytes::from_static(b"secret"),
        ),
    ]);
    if let Err(e) = widget.clipboard().set_content(Some(&provider)) {
        eprintln!("panora-gui: link not copied: {e}");
    }
}
