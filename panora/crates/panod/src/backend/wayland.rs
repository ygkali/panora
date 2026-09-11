// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! Native Wayland clipboard backend using the data-control protocols.
//!
//! Both `ext-data-control-v1` (wayland-protocols staging; GNOME 48+, KDE,
//! wlroots) and the older `zwlr-data-control-v1` (wlroots, KDE, Hyprland,
//! …) are supported; the ext variant is preferred when both exist.
//!
//! Capture is event driven: the compositor announces every new selection
//! with its MIME list *before* any payload is transferred, which is exactly
//! the TARGETS-first ordering ADR 0003 requires. Payloads are pulled through
//! a pipe only after the privacy engine allowed them. Offering data creates a
//! data source with every stored format and serves `send` requests on a
//! background thread until another client takes the selection.
//!
//! The protocol deliberately exposes no client identity, so `source_app` is
//! always `None` here; on GNOME the Shell extension supplies the app id and
//! the MIME secret-flag gate applies everywhere.

use async_trait::async_trait;
use panora_core::backend::{Capabilities, ClipboardBackend, ClipboardEvent};
use panora_core::error::{Error, Result};
use panora_core::model::{ClipboardData, MimePayload, Selection};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::os::fd::{AsFd, OwnedFd};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};
use wayland_client::backend::ObjectId;
use wayland_client::globals::{registry_queue_init, GlobalList, GlobalListContents};
use wayland_client::protocol::{wl_registry, wl_seat};
use wayland_client::{
    delegate_noop, event_created_child, Connection, Dispatch, EventQueue, Proxy, QueueHandle,
};
use wayland_protocols::ext::data_control::v1::client::{
    ext_data_control_device_v1 as ext_device, ext_data_control_manager_v1 as ext_manager,
    ext_data_control_offer_v1 as ext_offer, ext_data_control_source_v1 as ext_source,
};
use wayland_protocols_wlr::data_control::v1::client::{
    zwlr_data_control_device_v1 as wlr_device, zwlr_data_control_manager_v1 as wlr_manager,
    zwlr_data_control_offer_v1 as wlr_offer, zwlr_data_control_source_v1 as wlr_source,
};

/// Marker format added to our own offers so the watcher can tell a recall
/// apart from a user copy. Other clipboard tools ignore unknown types.
const RECALL_MARKER_MIME: &str = "application/x-panora-recall";
/// Idle timeout while a source writes a payload into our pipe (and while a
/// requestor drains ours).
const RECEIVE_TIMEOUT: Duration = Duration::from_secs(5);
/// How long a fresh session waits for the compositor's selection announcement.
const SETTLE_TIMEOUT: Duration = Duration::from_secs(2);
/// Hard cap on one received payload (the daemon applies the configured
/// limit afterwards; this only bounds memory against a hostile source).
const RECEIVE_CAP: usize = 256 * 1024 * 1024;

/// Native Wayland backend.
#[derive(Debug, Clone, Copy)]
pub struct WaylandBackend {
    primary: bool,
}

impl WaylandBackend {
    /// Connect and verify that the compositor offers a data-control protocol.
    pub fn connect() -> Result<Self> {
        if std::env::var_os("WAYLAND_DISPLAY").is_none() {
            return Err(Error::Backend("WAYLAND_DISPLAY is not set".into()));
        }
        let session = Session::new()?;
        info!(
            protocol = session.manager.protocol(),
            "wayland data-control available"
        );
        Ok(Self {
            primary: session.manager.supports_primary(),
        })
    }
}

#[async_trait]
impl ClipboardBackend for WaylandBackend {
    fn name(&self) -> &'static str {
        "wayland"
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            primary: self.primary,
            images: true,
            // Left to the compositor: Mutter and KWin keep clipboard content
            // after the source exits; a null selection cannot be told apart
            // from an intentional clear, so panod never re-offers here.
            persist: false,
            synthetic_paste: true,
            needs_bridge: false,
        }
    }

    async fn watch(&self, selection: Selection) -> Result<mpsc::Receiver<ClipboardEvent>> {
        let (sender, receiver) = mpsc::channel(64);
        let session = Session::new()?;
        std::thread::Builder::new()
            .name(format!("panora-wl-watch-{}", selection.as_str()))
            .spawn(move || watch_loop(session, selection, sender))
            .map_err(|e| Error::Backend(e.to_string()))?;
        Ok(receiver)
    }

    async fn read_targets(&self, selection: Selection) -> Result<Vec<String>> {
        tokio::task::spawn_blocking(move || {
            let mut session = Session::new()?;
            session.settle()?;
            Ok(session
                .state
                .current(selection)
                .map(|o| o.mimes.clone())
                .unwrap_or_default())
        })
        .await
        .map_err(|e| Error::Backend(e.to_string()))?
    }

    async fn read(&self, selection: Selection, mime: &str) -> Result<Vec<u8>> {
        let mime = mime.to_string();
        tokio::task::spawn_blocking(move || {
            let mut session = Session::new()?;
            session.settle()?;
            session.receive(selection, &mime)
        })
        .await
        .map_err(|e| Error::Backend(e.to_string()))?
    }

    async fn offer(&self, selection: Selection, data: ClipboardData) -> Result<()> {
        tokio::task::spawn_blocking(move || offer_blocking(selection, data.payloads))
            .await
            .map_err(|e| Error::Backend(e.to_string()))?
    }

    async fn synthetic_paste(&self) -> Result<()> {
        crate::paste::wayland_paste().await
    }
}

fn wlerr(e: impl std::fmt::Display) -> Error {
    Error::Backend(format!("wayland: {e}"))
}

// ---------------------------------------------------------------- objects

/// Either flavour of the data-control manager.
#[derive(Clone)]
enum Manager {
    Ext(ext_manager::ExtDataControlManagerV1),
    Wlr(wlr_manager::ZwlrDataControlManagerV1),
}

impl Manager {
    fn protocol(&self) -> &'static str {
        match self {
            Manager::Ext(_) => "ext-data-control-v1",
            Manager::Wlr(_) => "wlr-data-control-unstable-v1",
        }
    }

    fn supports_primary(&self) -> bool {
        match self {
            Manager::Ext(_) => true,
            Manager::Wlr(m) => m.version() >= 2,
        }
    }

    fn get_device(&self, seat: &wl_seat::WlSeat, qh: &QueueHandle<State>) -> Device {
        match self {
            Manager::Ext(m) => Device::Ext(m.get_data_device(seat, qh, ())),
            Manager::Wlr(m) => Device::Wlr(m.get_data_device(seat, qh, ())),
        }
    }

    fn create_source(&self, qh: &QueueHandle<State>) -> Source {
        match self {
            Manager::Ext(m) => Source::Ext(m.create_data_source(qh, ())),
            Manager::Wlr(m) => Source::Wlr(m.create_data_source(qh, ())),
        }
    }
}

enum Device {
    Ext(ext_device::ExtDataControlDeviceV1),
    Wlr(wlr_device::ZwlrDataControlDeviceV1),
}

impl Device {
    fn set_selection(&self, selection: Selection, source: &Source) {
        match (self, source, selection) {
            (Device::Ext(d), Source::Ext(s), Selection::Clipboard) => d.set_selection(Some(s)),
            (Device::Ext(d), Source::Ext(s), Selection::Primary) => {
                d.set_primary_selection(Some(s))
            }
            (Device::Wlr(d), Source::Wlr(s), Selection::Clipboard) => d.set_selection(Some(s)),
            (Device::Wlr(d), Source::Wlr(s), Selection::Primary) => {
                d.set_primary_selection(Some(s))
            }
            _ => {}
        }
    }
}

#[derive(Clone)]
enum Offer {
    Ext(ext_offer::ExtDataControlOfferV1),
    Wlr(wlr_offer::ZwlrDataControlOfferV1),
}

impl Offer {
    fn id(&self) -> ObjectId {
        match self {
            Offer::Ext(o) => o.id(),
            Offer::Wlr(o) => o.id(),
        }
    }

    fn receive(&self, mime: String, fd: std::os::fd::BorrowedFd<'_>) {
        match self {
            Offer::Ext(o) => o.receive(mime, fd),
            Offer::Wlr(o) => o.receive(mime, fd),
        }
    }

    fn destroy(&self) {
        match self {
            Offer::Ext(o) => o.destroy(),
            Offer::Wlr(o) => o.destroy(),
        }
    }
}

enum Source {
    Ext(ext_source::ExtDataControlSourceV1),
    Wlr(wlr_source::ZwlrDataControlSourceV1),
}

impl Source {
    fn offer(&self, mime: String) {
        match self {
            Source::Ext(s) => s.offer(mime),
            Source::Wlr(s) => s.offer(mime),
        }
    }

    fn destroy(&self) {
        match self {
            Source::Ext(s) => s.destroy(),
            Source::Wlr(s) => s.destroy(),
        }
    }
}

/// An offer the compositor announced, with the MIME list it advertised.
struct OfferInfo {
    offer: Offer,
    mimes: Vec<String>,
}

/// A selection change noticed by the device.
struct SelectionChange {
    selection: Selection,
    /// `None` when the selection became empty.
    mimes: Option<Vec<String>>,
}

// ------------------------------------------------------------------ state

#[derive(Default)]
struct State {
    offers: HashMap<ObjectId, OfferInfo>,
    clipboard: Option<ObjectId>,
    primary: Option<ObjectId>,
    /// Set once the compositor delivered the initial selection events.
    settled: bool,
    changes: Vec<SelectionChange>,
    finished: bool,
    /// Pending `send` requests for a source we own.
    sends: Vec<(String, OwnedFd)>,
    cancelled: bool,
}

impl State {
    fn current(&self, selection: Selection) -> Option<&OfferInfo> {
        let id = match selection {
            Selection::Clipboard => self.clipboard.as_ref()?,
            Selection::Primary => self.primary.as_ref()?,
        };
        self.offers.get(id)
    }

    fn register_offer(&mut self, offer: Offer) {
        self.offers.insert(
            offer.id(),
            OfferInfo {
                offer,
                mimes: Vec::new(),
            },
        );
    }

    fn add_mime(&mut self, id: &ObjectId, mime: String) {
        if let Some(info) = self.offers.get_mut(id) {
            if info.mimes.len() < panora_core::privacy::MAX_OFFERED_MIMES * 2 {
                info.mimes.push(mime);
            }
        }
    }

    fn set_selection(&mut self, selection: Selection, offer: Option<Offer>) {
        self.settled = true;
        let slot = match selection {
            Selection::Clipboard => &mut self.clipboard,
            Selection::Primary => &mut self.primary,
        };
        if let Some(old) = slot.take() {
            if Some(&old) != offer.as_ref().map(Offer::id).as_ref() {
                if let Some(info) = self.offers.remove(&old) {
                    info.offer.destroy();
                }
            }
        }
        let mimes = offer.as_ref().and_then(|o| {
            let id = o.id();
            *slot = Some(id.clone());
            self.offers.get(&id).map(|info| info.mimes.clone())
        });
        self.changes.push(SelectionChange { selection, mimes });
    }
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

delegate_noop!(State: ignore wl_seat::WlSeat);
delegate_noop!(State: ignore ext_manager::ExtDataControlManagerV1);
delegate_noop!(State: ignore wlr_manager::ZwlrDataControlManagerV1);

impl Dispatch<ext_device::ExtDataControlDeviceV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &ext_device::ExtDataControlDeviceV1,
        event: ext_device::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            ext_device::Event::DataOffer { id } => state.register_offer(Offer::Ext(id)),
            ext_device::Event::Selection { id } => {
                state.set_selection(Selection::Clipboard, id.map(Offer::Ext))
            }
            ext_device::Event::PrimarySelection { id } => {
                state.set_selection(Selection::Primary, id.map(Offer::Ext))
            }
            ext_device::Event::Finished => state.finished = true,
            _ => {}
        }
    }

    event_created_child!(State, ext_device::ExtDataControlDeviceV1, [
        ext_device::EVT_DATA_OFFER_OPCODE => (ext_offer::ExtDataControlOfferV1, ()),
    ]);
}

impl Dispatch<wlr_device::ZwlrDataControlDeviceV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &wlr_device::ZwlrDataControlDeviceV1,
        event: wlr_device::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wlr_device::Event::DataOffer { id } => state.register_offer(Offer::Wlr(id)),
            wlr_device::Event::Selection { id } => {
                state.set_selection(Selection::Clipboard, id.map(Offer::Wlr))
            }
            wlr_device::Event::PrimarySelection { id } => {
                state.set_selection(Selection::Primary, id.map(Offer::Wlr))
            }
            wlr_device::Event::Finished => state.finished = true,
            _ => {}
        }
    }

    event_created_child!(State, wlr_device::ZwlrDataControlDeviceV1, [
        wlr_device::EVT_DATA_OFFER_OPCODE => (wlr_offer::ZwlrDataControlOfferV1, ()),
    ]);
}

impl Dispatch<ext_offer::ExtDataControlOfferV1, ()> for State {
    fn event(
        state: &mut Self,
        offer: &ext_offer::ExtDataControlOfferV1,
        event: ext_offer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let ext_offer::Event::Offer { mime_type } = event {
            state.add_mime(&offer.id(), mime_type);
        }
    }
}

impl Dispatch<wlr_offer::ZwlrDataControlOfferV1, ()> for State {
    fn event(
        state: &mut Self,
        offer: &wlr_offer::ZwlrDataControlOfferV1,
        event: wlr_offer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wlr_offer::Event::Offer { mime_type } = event {
            state.add_mime(&offer.id(), mime_type);
        }
    }
}

impl Dispatch<ext_source::ExtDataControlSourceV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &ext_source::ExtDataControlSourceV1,
        event: ext_source::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            ext_source::Event::Send { mime_type, fd } => state.sends.push((mime_type, fd)),
            ext_source::Event::Cancelled => state.cancelled = true,
            _ => {}
        }
    }
}

impl Dispatch<wlr_source::ZwlrDataControlSourceV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &wlr_source::ZwlrDataControlSourceV1,
        event: wlr_source::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wlr_source::Event::Send { mime_type, fd } => state.sends.push((mime_type, fd)),
            wlr_source::Event::Cancelled => state.cancelled = true,
            _ => {}
        }
    }
}

// ---------------------------------------------------------------- session

/// One connection with a bound manager, seat and data device.
struct Session {
    conn: Connection,
    queue: EventQueue<State>,
    qh: QueueHandle<State>,
    manager: Manager,
    #[allow(dead_code)]
    seat: wl_seat::WlSeat,
    device: Device,
    state: State,
}

impl Session {
    fn new() -> Result<Self> {
        let conn = Connection::connect_to_env().map_err(wlerr)?;
        let (globals, queue): (GlobalList, EventQueue<State>) =
            registry_queue_init(&conn).map_err(wlerr)?;
        let qh = queue.handle();
        let manager = if let Ok(m) =
            globals.bind::<ext_manager::ExtDataControlManagerV1, State, ()>(&qh, 1..=1, ())
        {
            Manager::Ext(m)
        } else if let Ok(m) =
            globals.bind::<wlr_manager::ZwlrDataControlManagerV1, State, ()>(&qh, 1..=2, ())
        {
            Manager::Wlr(m)
        } else {
            return Err(Error::Backend(
                "wayland: compositor offers neither ext-data-control-v1 nor \
                 wlr-data-control-v1 (on GNOME < 48 install the Panora Shell extension)"
                    .into(),
            ));
        };
        let seat = globals
            .bind::<wl_seat::WlSeat, State, ()>(&qh, 1..=1, ())
            .map_err(|e| wlerr(format!("no wl_seat: {e}")))?;
        let device = manager.get_device(&seat, &qh);
        Ok(Self {
            conn,
            queue,
            qh,
            manager,
            seat,
            device,
            state: State::default(),
        })
    }

    /// Dispatch until the compositor has announced the current selections.
    /// Both protocols send `selection` right after `get_data_device`, so a
    /// round trip normally suffices; the bounded loop guards against a
    /// compositor that never answers.
    fn settle(&mut self) -> Result<()> {
        self.queue.roundtrip(&mut self.state).map_err(wlerr)?;
        let deadline = Instant::now() + SETTLE_TIMEOUT;
        while !self.state.settled {
            if Instant::now() >= deadline {
                return Err(Error::Backend(
                    "wayland: compositor did not announce the selection".into(),
                ));
            }
            dispatch_with_timeout(&mut self.queue, &mut self.state, Duration::from_millis(200))?;
        }
        Ok(())
    }

    /// Pull one MIME payload of the current selection through a pipe.
    fn receive(&mut self, selection: Selection, mime: &str) -> Result<Vec<u8>> {
        let offer = self
            .state
            .current(selection)
            .ok_or_else(|| Error::Backend("wayland: selection is empty".into()))?;
        if !offer.mimes.iter().any(|m| m == mime) {
            return Err(Error::Backend(format!("wayland: {mime} is not offered")));
        }
        let (reader, writer) = rustix::pipe::pipe().map_err(wlerr)?;
        offer.offer.receive(mime.to_string(), writer.as_fd());
        self.conn.flush().map_err(wlerr)?;
        drop(writer);
        read_pipe(reader)
    }
}

/// Read until EOF with an idle timeout so a stalled source cannot hang the
/// daemon's blocking pool.
fn read_pipe(fd: OwnedFd) -> Result<Vec<u8>> {
    use rustix::event::{poll, PollFd, PollFlags, Timespec};
    let timeout = Timespec {
        tv_sec: RECEIVE_TIMEOUT.as_secs() as _,
        tv_nsec: 0,
    };
    let mut file = std::fs::File::from(fd);
    let mut out = Vec::new();
    let mut chunk = vec![0u8; 64 * 1024];
    loop {
        let ready = {
            let mut fds = [PollFd::new(&file, PollFlags::IN)];
            match poll(&mut fds, Some(&timeout)) {
                Ok(n) => n,
                Err(rustix::io::Errno::INTR) => continue,
                Err(e) => return Err(wlerr(e)),
            }
        };
        if ready == 0 {
            return Err(Error::Backend(
                "wayland: source stopped sending data".into(),
            ));
        }
        let n = file.read(&mut chunk)?;
        if n == 0 {
            return Ok(out);
        }
        out.extend_from_slice(&chunk[..n]);
        if out.len() > RECEIVE_CAP {
            return Err(Error::TooLarge {
                size: out.len(),
                limit: RECEIVE_CAP,
            });
        }
    }
}

/// Watcher thread: forward every selection change to the daemon.
fn watch_loop(mut session: Session, selection: Selection, sender: mpsc::Sender<ClipboardEvent>) {
    // The initial selection (whatever was on the clipboard before panod
    // started) is reported too, so history picks it up after a restart.
    loop {
        if let Err(e) = session.queue.blocking_dispatch(&mut session.state) {
            warn!(error = %e, "wayland watcher connection lost");
            return;
        }
        if session.state.finished {
            warn!("wayland data device finished; capture stopped");
            return;
        }
        for change in session.state.changes.drain(..) {
            if change.selection != selection {
                continue;
            }
            let event = match change.mimes {
                // A null selection is both "owner exited" and "explicitly
                // cleared" (password managers do the latter on purpose), so
                // nothing is re-offered here; compositors with clipboard
                // persistence (Mutter, KWin) keep content on their own.
                None => continue,
                Some(mimes) => {
                    if mimes.iter().any(|m| m == RECALL_MARKER_MIME) {
                        debug!("wayland: ignoring our own recall offer");
                        continue;
                    }
                    let mimes: Vec<String> = mimes
                        .into_iter()
                        .map(|m| m.trim().to_string())
                        .filter(|m| !m.is_empty())
                        .collect();
                    if mimes.is_empty() {
                        continue;
                    }
                    debug!(count = mimes.len(), "wayland clipboard change");
                    ClipboardEvent::changed(selection, mimes, None)
                }
            };
            if sender.blocking_send(event).is_err() {
                return;
            }
        }
    }
}

/// Become the selection source and serve `send` requests until replaced.
fn offer_blocking(selection: Selection, payloads: Vec<MimePayload>) -> Result<()> {
    if payloads.is_empty() {
        return Err(Error::Backend("nothing to offer".into()));
    }
    let mut session = Session::new()?;
    if selection == Selection::Primary && !session.manager.supports_primary() {
        return Err(Error::Backend(
            "wayland: compositor has no primary selection support".into(),
        ));
    }
    let source = session.manager.create_source(&session.qh);
    let mut advertised: Vec<String> = Vec::new();
    for payload in &payloads {
        advertised.push(payload.mime.clone());
    }
    // Text aliases so every toolkit finds a format it understands.
    if payloads.iter().any(MimePayload::is_text) {
        for alias in [
            "text/plain;charset=utf-8",
            "text/plain",
            "UTF8_STRING",
            "STRING",
            "TEXT",
        ] {
            if !advertised.iter().any(|m| m == alias) {
                advertised.push(alias.to_string());
            }
        }
    }
    advertised.push(RECALL_MARKER_MIME.to_string());
    for mime in &advertised {
        source.offer(mime.clone());
    }
    session.device.set_selection(selection, &source);
    session.queue.roundtrip(&mut session.state).map_err(wlerr)?;
    if session.state.cancelled {
        return Err(Error::Backend(
            "wayland: selection offer was rejected".into(),
        ));
    }

    std::thread::Builder::new()
        .name("panora-wl-owner".into())
        .spawn(move || serve_source(session, source, payloads))
        .map_err(|e| Error::Backend(e.to_string()))?;
    Ok(())
}

fn serve_source(mut session: Session, source: Source, payloads: Vec<MimePayload>) {
    let payloads = std::sync::Arc::new(payloads);
    loop {
        // Serve first: `send` requests that arrived during the set_selection
        // round trip are already queued, and blocking now would leave the
        // requestor waiting on our pipe.
        for (mime, fd) in session.state.sends.drain(..) {
            let payloads = payloads.clone();
            // Writing blocks until the receiver reads; keep the dispatch
            // loop responsive by writing on its own thread.
            std::thread::spawn(move || {
                if let Some(payload) = payload_for(&payloads, &mime) {
                    if let Err(e) = write_pipe(fd, &payload.data) {
                        debug!(error = %e, mime, "wayland: send aborted");
                    }
                }
            });
        }
        session.state.changes.clear();
        if session.state.cancelled {
            debug!("wayland: selection taken by another client");
            source.destroy();
            let _ = session.conn.flush();
            return;
        }
        if let Err(e) = session.queue.blocking_dispatch(&mut session.state) {
            debug!(error = %e, "wayland owner connection closed");
            return;
        }
    }
}

/// Write a payload into a requestor's pipe without blocking forever on a
/// reader that stalls: the descriptor is switched to non-blocking mode and
/// every write waits for writability with an idle timeout.
fn write_pipe(fd: OwnedFd, data: &[u8]) -> Result<()> {
    use rustix::event::{poll, PollFd, PollFlags, Timespec};
    use rustix::fs::{fcntl_setfl, OFlags};
    let timeout = Timespec {
        tv_sec: RECEIVE_TIMEOUT.as_secs() as _,
        tv_nsec: 0,
    };
    fcntl_setfl(&fd, OFlags::NONBLOCK).map_err(wlerr)?;
    let mut file = std::fs::File::from(fd);
    let mut offset = 0;
    while offset < data.len() {
        let ready = {
            let mut fds = [PollFd::new(&file, PollFlags::OUT)];
            match poll(&mut fds, Some(&timeout)) {
                Ok(n) => n,
                Err(rustix::io::Errno::INTR) => continue,
                Err(e) => return Err(wlerr(e)),
            }
        };
        if ready == 0 {
            return Err(Error::Backend("wayland: requestor stopped reading".into()));
        }
        match file.write(&data[offset..]) {
            Ok(0) => return Err(Error::Backend("wayland: requestor closed the pipe".into())),
            Ok(n) => offset += n,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

/// Dispatch pending events, then wait up to `timeout` for more. Returns
/// after at most one socket read so callers can check deadlines.
fn dispatch_with_timeout(
    queue: &mut EventQueue<State>,
    state: &mut State,
    timeout: Duration,
) -> Result<()> {
    use rustix::event::{poll, PollFd, PollFlags, Timespec};
    queue.dispatch_pending(state).map_err(wlerr)?;
    queue.flush().map_err(wlerr)?;
    let Some(guard) = queue.prepare_read() else {
        // Events are already queued; the next dispatch_pending handles them.
        return Ok(());
    };
    let wait = Timespec {
        tv_sec: timeout.as_secs() as _,
        tv_nsec: timeout.subsec_nanos() as _,
    };
    let ready = {
        let fd = guard.connection_fd();
        let mut fds = [PollFd::new(&fd, PollFlags::IN)];
        match poll(&mut fds, Some(&wait)) {
            Ok(n) => n,
            Err(rustix::io::Errno::INTR) => 0,
            Err(e) => return Err(wlerr(e)),
        }
    };
    if ready > 0 {
        match guard.read() {
            Ok(_) => {}
            Err(wayland_client::backend::WaylandError::Io(e))
                if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(e) => return Err(wlerr(e)),
        }
        queue.dispatch_pending(state).map_err(wlerr)?;
    }
    Ok(())
}

/// The payload to serve for a requested MIME, honouring text aliases.
fn payload_for<'a>(payloads: &'a [MimePayload], mime: &str) -> Option<&'a MimePayload> {
    if let Some(p) = payloads.iter().find(|p| p.mime == mime) {
        return Some(p);
    }
    if mime == RECALL_MARKER_MIME {
        return None;
    }
    if panora_core::model::is_text_mime(mime) {
        for preferred in panora_core::model::TEXT_MIMES {
            if let Some(p) = payloads.iter().find(|p| p.mime == *preferred) {
                return Some(p);
            }
        }
        return payloads.iter().find(|p| p.is_text());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_aliases_resolve_to_best_text_payload() {
        let payloads = vec![
            MimePayload::new("text/html", "<b>x</b>"),
            MimePayload::new("text/plain;charset=utf-8", "x"),
        ];
        assert_eq!(payload_for(&payloads, "STRING").unwrap().data, b"x");
        assert_eq!(
            payload_for(&payloads, "text/html").unwrap().data,
            b"<b>x</b>"
        );
        assert!(payload_for(&payloads, "image/png").is_none());
        assert!(payload_for(&payloads, RECALL_MARKER_MIME).is_none());
    }
}
