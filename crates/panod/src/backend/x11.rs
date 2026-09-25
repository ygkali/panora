// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! Native X11 clipboard backend on top of `x11rb`.
//!
//! * Change detection uses the XFIXES `SelectionNotify` event, so there is
//!   no polling and no helper process.
//! * TARGETS are read before any payload (ADR 0003); payloads are fetched
//!   with `ConvertSelection`, including `INCR` transfers for large data.
//! * `offer` makes panod the selection owner itself and answers
//!   `SelectionRequest`s for every stored format at once (text + HTML +
//!   image), with `INCR` for payloads above the request-size limit.
//! * When the owning application exits the daemon is told (`OwnerGone`) so
//!   it can re-offer the last entry: X11 has no clipboard persistence.
//! * Instant paste uses the XTEST extension.

use async_trait::async_trait;
use panora_core::backend::{Capabilities, ClipboardBackend, ClipboardEvent};
use panora_core::error::{Error, Result};
use panora_core::model::{ClipboardData, MimePayload, Selection, TEXT_MIMES};
use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tracing::{debug, warn};
use x11rb::connection::{Connection, RequestConnection as _};
use x11rb::protocol::xfixes::{self, ConnectionExt as _, SelectionEventMask};
use x11rb::protocol::xproto::{
    Atom, AtomEnum, ChangeWindowAttributesAux, ConnectionExt as _, CreateWindowAux, EventMask,
    GetPropertyType, PropMode, Property, SelectionNotifyEvent, SelectionRequestEvent, Timestamp,
    Window, WindowClass, SELECTION_NOTIFY_EVENT,
};
use x11rb::protocol::xtest::ConnectionExt as _;
use x11rb::protocol::Event;
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

/// How far up the window tree to look for a WM_CLASS before giving up.
const WM_CLASS_DEPTH: usize = 8;
/// How long to wait for a selection owner to answer one conversion.
const CONVERT_TIMEOUT: Duration = Duration::from_secs(3);
/// How long the daemon waits for its own owner thread to take the selection.
const OWNER_TIMEOUT: Duration = Duration::from_secs(3);
/// Payloads above this size are served with the INCR protocol.
const INCR_THRESHOLD: usize = 256 * 1024;
/// Upper bound for one property read (in 32-bit units).
const MAX_PROPERTY_LEN: u32 = u32::MAX / 4;

x11rb::atom_manager! {
    pub Atoms: AtomsCookie {
        CLIPBOARD,
        TARGETS,
        TIMESTAMP,
        MULTIPLE,
        INCR,
        UTF8_STRING,
        TEXT,
        STRING,
        _NET_ACTIVE_WINDOW,
        _NET_WM_NAME,
        _PANORA_OWNER,
        _PANORA_XFER,
    }
}

/// Native X11 backend.
#[derive(Debug, Clone, Copy, Default)]
pub struct X11Backend;

impl X11Backend {
    /// Verify that an X server with XFIXES is reachable.
    pub fn connect() -> Result<Self> {
        if std::env::var_os("DISPLAY").is_none() {
            return Err(Error::Backend("DISPLAY is not set".into()));
        }
        let (conn, _) = x11rb::connect(None).map_err(x11err)?;
        let version = conn
            .xfixes_query_version(5, 0)
            .map_err(x11err)?
            .reply()
            .map_err(x11err)?;
        if version.major_version < 1 {
            return Err(Error::Backend("XFIXES extension unavailable".into()));
        }
        Ok(Self)
    }
}

#[async_trait]
impl ClipboardBackend for X11Backend {
    fn name(&self) -> &'static str {
        "x11"
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            primary: true,
            images: true,
            persist: true,
            synthetic_paste: true,
            // `_NET_ACTIVE_WINDOW` → `WM_CLASS`; see `focused_app`.
            source_app: true,
            needs_bridge: false,
        }
    }

    async fn watch(&self, selection: Selection) -> Result<mpsc::Receiver<ClipboardEvent>> {
        let (sender, receiver) = mpsc::channel(64);
        // Fail early if the watcher cannot even connect, instead of dying
        // silently on a thread nobody observes.
        let watcher = Watcher::new(selection)?;
        std::thread::Builder::new()
            .name(format!("panora-x11-watch-{}", selection.as_str()))
            .spawn(move || watcher.run(sender))
            .map_err(|e| Error::Backend(e.to_string()))?;
        Ok(receiver)
    }

    async fn read_targets(&self, selection: Selection) -> Result<Vec<String>> {
        tokio::task::spawn_blocking(move || {
            let reader = Reader::new()?;
            reader.targets(selection)
        })
        .await
        .map_err(|e| Error::Backend(e.to_string()))?
    }

    async fn read(&self, selection: Selection, mime: &str) -> Result<Vec<u8>> {
        let mime = mime.to_string();
        tokio::task::spawn_blocking(move || {
            let reader = Reader::new()?;
            reader.payload(selection, &mime)
        })
        .await
        .map_err(|e| Error::Backend(e.to_string()))?
    }

    async fn offer(&self, selection: Selection, data: ClipboardData) -> Result<()> {
        tokio::task::spawn_blocking(move || Owner::spawn(selection, data.payloads))
            .await
            .map_err(|e| Error::Backend(e.to_string()))?
    }

    async fn synthetic_paste(&self) -> Result<()> {
        tokio::task::spawn_blocking(xtest_paste)
            .await
            .map_err(|e| Error::Backend(e.to_string()))?
    }

    async fn clear(&self, selection: Selection) -> Result<()> {
        tokio::task::spawn_blocking(move || release_selection(selection))
            .await
            .map_err(|e| Error::Backend(e.to_string()))?
    }
}

/// Release ownership of `selection` (CAP-07). Any client may set a
/// selection's owner to `NONE` regardless of who currently holds it, so the
/// caller is responsible for confirming (via `read_targets`) that this is
/// still the content it put there before calling this.
fn release_selection(selection: Selection) -> Result<()> {
    let client = Client::new()?;
    let sel = selection_atom(&client.atoms, selection);
    let timestamp = client.server_time()?;
    // `.check()` round-trips instead of just flushing the request to the
    // socket: without it, this can return before the X server has actually
    // processed the ownership change, so a caller that immediately reads
    // TARGETS again could still see the old owner.
    client
        .conn
        .set_selection_owner(x11rb::NONE, sel, timestamp)
        .map_err(x11err)?
        .check()
        .map_err(x11err)?;
    Ok(())
}

fn x11err(e: impl std::fmt::Display) -> Error {
    Error::Backend(format!("x11: {e}"))
}

fn selection_atom(atoms: &Atoms, selection: Selection) -> Atom {
    match selection {
        Selection::Clipboard => atoms.CLIPBOARD,
        Selection::Primary => AtomEnum::PRIMARY.into(),
    }
}

/// A connection plus a hidden window used for selection transfers.
struct Client {
    conn: RustConnection,
    root: Window,
    window: Window,
    atoms: Atoms,
    /// Events received while waiting for a specific reply; they are
    /// replayed by `next_event` so no owner change is lost during a
    /// conversion.
    pending: RefCell<VecDeque<Event>>,
}

impl Client {
    fn new() -> Result<Self> {
        let (conn, screen_num) = x11rb::connect(None).map_err(x11err)?;
        let screen = conn
            .setup()
            .roots
            .get(screen_num)
            .ok_or_else(|| Error::Backend("x11: no screen".into()))?;
        let root = screen.root;
        let window = conn.generate_id().map_err(x11err)?;
        conn.create_window(
            x11rb::COPY_DEPTH_FROM_PARENT,
            window,
            root,
            -10,
            -10,
            1,
            1,
            0,
            WindowClass::INPUT_OUTPUT,
            x11rb::COPY_FROM_PARENT,
            &CreateWindowAux::new().event_mask(EventMask::PROPERTY_CHANGE),
        )
        .map_err(x11err)?;
        let atoms = Atoms::new(&conn).map_err(x11err)?.reply().map_err(x11err)?;
        conn.flush().map_err(x11err)?;
        Ok(Self {
            conn,
            root,
            window,
            atoms,
            pending: RefCell::new(VecDeque::new()),
        })
    }

    /// Next event: replayed ones first, then a blocking read.
    fn next_event(&self) -> Result<Event> {
        if let Some(event) = self.pending.borrow_mut().pop_front() {
            return Ok(event);
        }
        self.conn.wait_for_event().map_err(x11err)
    }

    fn intern(&self, name: &str) -> Result<Atom> {
        Ok(self
            .conn
            .intern_atom(false, name.as_bytes())
            .map_err(x11err)?
            .reply()
            .map_err(x11err)?
            .atom)
    }

    fn atom_name(&self, atom: Atom) -> Result<String> {
        let reply = self
            .conn
            .get_atom_name(atom)
            .map_err(x11err)?
            .reply()
            .map_err(x11err)?;
        Ok(String::from_utf8_lossy(&reply.name).into_owned())
    }

    /// Wait for one event matching `accept` until the deadline. Events that
    /// do not match are queued for `next_event`.
    fn wait_event<T>(
        &self,
        deadline: Instant,
        mut accept: impl FnMut(&Event) -> Option<T>,
    ) -> Result<T> {
        loop {
            if let Some(event) = self.conn.poll_for_event().map_err(x11err)? {
                if let Some(value) = accept(&event) {
                    return Ok(value);
                }
                self.pending.borrow_mut().push_back(event);
                continue;
            }
            if Instant::now() >= deadline {
                return Err(Error::Backend("x11: selection owner did not answer".into()));
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    /// Current server time, obtained through a property round trip.
    fn server_time(&self) -> Result<Timestamp> {
        self.conn
            .change_property8(
                PropMode::APPEND,
                self.window,
                self.atoms._PANORA_XFER,
                AtomEnum::STRING,
                &[],
            )
            .map_err(x11err)?;
        self.conn.flush().map_err(x11err)?;
        let window = self.window;
        let property = self.atoms._PANORA_XFER;
        self.wait_event(Instant::now() + CONVERT_TIMEOUT, |event| match event {
            Event::PropertyNotify(e) if e.window == window && e.atom == property => Some(e.time),
            _ => None,
        })
    }

    /// Ask the owner of `selection` to convert `target` into our transfer
    /// property and return the property's type and bytes (INCR resolved).
    fn convert(&self, selection: Atom, target: Atom) -> Result<(Atom, u8, Vec<u8>)> {
        let property = self.atoms._PANORA_XFER;
        // A real timestamp (ICCCM 2.6.2) doubles as the request id: a late
        // reply to an earlier, timed-out conversion carries a different time
        // and is not mistaken for this one. Deleting the property first keeps
        // a stale value from being read as the answer.
        let time = self.server_time()?;
        self.conn
            .delete_property(self.window, property)
            .map_err(x11err)?;
        self.conn
            .convert_selection(self.window, selection, target, property, time)
            .map_err(x11err)?;
        self.conn.flush().map_err(x11err)?;
        let window = self.window;
        let deadline = Instant::now() + CONVERT_TIMEOUT;
        let notify: SelectionNotifyEvent = self.wait_event(deadline, |event| match event {
            Event::SelectionNotify(e)
                if e.requestor == window
                    && e.selection == selection
                    && e.target == target
                    // Owners echo the request time; a few reply CurrentTime.
                    && (e.time == time || e.time == x11rb::CURRENT_TIME) =>
            {
                Some(*e)
            }
            _ => None,
        })?;
        if notify.property == x11rb::NONE {
            return Err(Error::Backend(
                "x11: selection owner refused the conversion".into(),
            ));
        }
        let reply = self
            .conn
            .get_property(
                true,
                window,
                property,
                GetPropertyType::ANY,
                0,
                MAX_PROPERTY_LEN,
            )
            .map_err(x11err)?
            .reply()
            .map_err(x11err)?;
        if reply.type_ != self.atoms.INCR {
            return Ok((reply.type_, reply.format, reply.value));
        }

        // INCR: the owner streams chunks into the property; every delete of
        // ours asks for the next one, and an empty chunk ends the transfer.
        self.conn.flush().map_err(x11err)?;
        let mut data = Vec::new();
        let mut type_ = self.atoms.INCR;
        let mut format = 8;
        loop {
            let deadline = Instant::now() + CONVERT_TIMEOUT;
            self.wait_event(deadline, |event| match event {
                Event::PropertyNotify(e)
                    if e.window == window
                        && e.atom == property
                        && e.state == Property::NEW_VALUE =>
                {
                    Some(())
                }
                _ => None,
            })?;
            let chunk = self
                .conn
                .get_property(
                    true,
                    window,
                    property,
                    GetPropertyType::ANY,
                    0,
                    MAX_PROPERTY_LEN,
                )
                .map_err(x11err)?
                .reply()
                .map_err(x11err)?;
            self.conn.flush().map_err(x11err)?;
            if chunk.value.is_empty() {
                break;
            }
            type_ = chunk.type_;
            format = chunk.format;
            data.extend_from_slice(&chunk.value);
        }
        Ok((type_, format, data))
    }
}

/// Reads TARGETS and payloads from whoever owns a selection.
struct Reader {
    client: Client,
}

impl Reader {
    fn new() -> Result<Self> {
        Ok(Self {
            client: Client::new()?,
        })
    }

    fn targets(&self, selection: Selection) -> Result<Vec<String>> {
        let c = &self.client;
        let sel = selection_atom(&c.atoms, selection);
        if c.conn
            .get_selection_owner(sel)
            .map_err(x11err)?
            .reply()
            .map_err(x11err)?
            .owner
            == x11rb::NONE
        {
            return Ok(Vec::new());
        }
        let (type_, format, value) = c.convert(sel, c.atoms.TARGETS)?;
        if type_ != Atom::from(AtomEnum::ATOM) || format != 32 {
            return Ok(Vec::new());
        }
        let cookies: Vec<_> = value
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| Atom::from_ne_bytes(*b))
            .filter(|&a| a != x11rb::NONE)
            .map(|a| c.conn.get_atom_name(a))
            .collect::<std::result::Result<_, _>>()
            .map_err(x11err)?;
        let mut names = Vec::with_capacity(cookies.len());
        for cookie in cookies {
            if let Ok(reply) = cookie.reply() {
                names.push(String::from_utf8_lossy(&reply.name).into_owned());
            }
        }
        Ok(names)
    }

    fn payload(&self, selection: Selection, mime: &str) -> Result<Vec<u8>> {
        let c = &self.client;
        let sel = selection_atom(&c.atoms, selection);
        let target = c.intern(mime)?;
        let (_, _, value) = c.convert(sel, target)?;
        Ok(value)
    }
}

/// Watches one selection with XFIXES and reports owner changes.
struct Watcher {
    client: Client,
    selection: Selection,
}

impl Watcher {
    fn new(selection: Selection) -> Result<Self> {
        let client = Client::new()?;
        let sel = selection_atom(&client.atoms, selection);
        client
            .conn
            .xfixes_query_version(5, 0)
            .map_err(x11err)?
            .reply()
            .map_err(x11err)?;
        client
            .conn
            .xfixes_select_selection_input(
                client.window,
                sel,
                SelectionEventMask::SET_SELECTION_OWNER
                    | SelectionEventMask::SELECTION_WINDOW_DESTROY
                    | SelectionEventMask::SELECTION_CLIENT_CLOSE,
            )
            .map_err(x11err)?;
        client.conn.flush().map_err(x11err)?;
        Ok(Self { client, selection })
    }

    fn run(self, sender: mpsc::Sender<ClipboardEvent>) {
        let mut last: Option<(Window, Timestamp)> = None;
        // Report whatever is on the clipboard already, so history picks it
        // up after a (re)start just like the Wayland backend does.
        if let Some(event) = self.initial_state() {
            if sender.blocking_send(event).is_err() {
                return;
            }
        }
        loop {
            let event = match self.client.next_event() {
                Ok(event) => event,
                Err(e) => {
                    warn!(error = %e, "x11 watcher connection lost");
                    return;
                }
            };
            let Event::XfixesSelectionNotify(notify) = event else {
                continue;
            };
            let event = match notify.subtype {
                xfixes::SelectionEvent::SET_SELECTION_OWNER => {
                    if notify.owner == x11rb::NONE {
                        continue;
                    }
                    // Chrome and GTK set the owner more than once per copy;
                    // the selection timestamp identifies one user action.
                    let identity = (notify.owner, notify.selection_timestamp);
                    if last == Some(identity) {
                        continue;
                    }
                    last = Some(identity);
                    if self.is_own_window(notify.owner) {
                        debug!("x11: ignoring our own selection ownership");
                        continue;
                    }
                    let offered = match self.targets_of(notify.selection) {
                        Ok(t) if !t.is_empty() => t,
                        Ok(_) => continue,
                        Err(e) => {
                            debug!(error = %e, "x11: TARGETS read failed");
                            continue;
                        }
                    };
                    let source_app = focused_app(&self.client);
                    debug!(?source_app, count = offered.len(), "x11 clipboard change");
                    ClipboardEvent::changed(self.selection, offered, source_app)
                        .with_title(focused_title(&self.client))
                }
                xfixes::SelectionEvent::SELECTION_WINDOW_DESTROY
                | xfixes::SelectionEvent::SELECTION_CLIENT_CLOSE => {
                    last = None;
                    ClipboardEvent::owner_gone(self.selection)
                }
                _ => continue,
            };
            if sender.blocking_send(event).is_err() {
                return;
            }
        }
    }

    /// The selection as found at startup, if some other client owns it.
    fn initial_state(&self) -> Option<ClipboardEvent> {
        let sel = selection_atom(&self.client.atoms, self.selection);
        let owner = self
            .client
            .conn
            .get_selection_owner(sel)
            .ok()?
            .reply()
            .ok()?
            .owner;
        if owner == x11rb::NONE || self.is_own_window(owner) {
            return None;
        }
        let offered = self.targets_of(sel).ok().filter(|t| !t.is_empty())?;
        Some(ClipboardEvent::changed(
            self.selection,
            offered,
            focused_app(&self.client),
        ))
    }

    fn targets_of(&self, selection: Atom) -> Result<Vec<String>> {
        let c = &self.client;
        let (type_, format, value) = c.convert(selection, c.atoms.TARGETS)?;
        if type_ != Atom::from(AtomEnum::ATOM) || format != 32 {
            return Ok(Vec::new());
        }
        let mut names = Vec::new();
        for chunk in value.as_chunks::<4>().0 {
            let atom = Atom::from_ne_bytes(*chunk);
            if atom == x11rb::NONE {
                continue;
            }
            if let Ok(name) = c.atom_name(atom) {
                names.push(name);
            }
        }
        Ok(names)
    }

    /// Our owner windows carry `_PANORA_OWNER`, so a recall never loops
    /// back into the capture path.
    fn is_own_window(&self, window: Window) -> bool {
        self.client
            .conn
            .get_property(
                false,
                window,
                self.client.atoms._PANORA_OWNER,
                AtomEnum::CARDINAL,
                0,
                1,
            )
            .ok()
            .and_then(|c| c.reply().ok())
            .map(|r| r.value_len > 0)
            .unwrap_or(false)
    }
}

/// Serves our payloads to other applications while we own a selection.
struct Owner {
    client: Client,
    selection: Atom,
    payloads: Vec<MimePayload>,
    /// Target atom -> payload index.
    targets: HashMap<Atom, usize>,
    /// TARGETS reply (atoms we advertise).
    advertised: Vec<Atom>,
    timestamp: Timestamp,
    /// In-flight INCR transfers keyed by (requestor, property).
    incr: HashMap<(Window, Atom), IncrTransfer>,
    chunk: usize,
}

struct IncrTransfer {
    target: Atom,
    payload: usize,
    offset: usize,
    /// Last activity; a requestor that stops deleting the property is
    /// dropped after `INCR_TIMEOUT` instead of pinning the transfer forever.
    last_activity: Instant,
}

/// A requestor that does not make progress for this long is abandoned.
const INCR_TIMEOUT: Duration = Duration::from_secs(30);

/// Latin-1 encoding for the legacy `STRING` target; characters outside the
/// range become `?` like the reference toolkits do.
fn latin1_bytes(text: &str) -> Vec<u8> {
    text.chars()
        .map(u32::from)
        .map(|c| if c <= 0xff { c as u8 } else { b'?' })
        .collect()
}

impl Owner {
    /// Take ownership on a dedicated thread and return once the X server
    /// confirms it. The thread keeps serving until another owner appears.
    fn spawn(selection: Selection, payloads: Vec<MimePayload>) -> Result<()> {
        if payloads.is_empty() {
            return Err(Error::Backend("nothing to offer".into()));
        }
        let (tx, rx) = std::sync::mpsc::channel::<Result<()>>();
        std::thread::Builder::new()
            .name("panora-x11-owner".into())
            .spawn(move || {
                let owner = match Owner::new(selection, payloads) {
                    Ok(owner) => owner,
                    Err(e) => {
                        let _ = tx.send(Err(e));
                        return;
                    }
                };
                let _ = tx.send(Ok(()));
                owner.serve();
            })
            .map_err(|e| Error::Backend(e.to_string()))?;
        rx.recv_timeout(OWNER_TIMEOUT)
            .map_err(|_| Error::Backend("x11: owner thread did not start".into()))?
    }

    fn new(selection: Selection, mut payloads: Vec<MimePayload>) -> Result<Self> {
        let client = Client::new()?;
        let sel = selection_atom(&client.atoms, selection);
        client
            .conn
            .change_property32(
                PropMode::REPLACE,
                client.window,
                client.atoms._PANORA_OWNER,
                AtomEnum::CARDINAL,
                &[1],
            )
            .map_err(x11err)?;

        let mut targets = HashMap::new();
        let mut advertised = vec![client.atoms.TARGETS, client.atoms.TIMESTAMP];
        for (index, payload) in payloads.iter().enumerate() {
            let atom = client.intern(&payload.mime)?;
            targets.entry(atom).or_insert(index);
            advertised.push(atom);
        }
        // Legacy text targets map to the best text payload so old
        // toolkits and terminals can paste too. STRING is Latin-1 by
        // definition (ICCCM), so it gets a transcoded copy.
        if let Some(text_index) = payloads.iter().position(MimePayload::is_text) {
            let best = TEXT_MIMES
                .iter()
                .find_map(|m| payloads.iter().position(|p| p.mime == *m))
                .unwrap_or(text_index);
            for alias in [
                "UTF8_STRING",
                "text/plain;charset=utf-8",
                "text/plain",
                "TEXT",
            ] {
                let atom = client.intern(alias)?;
                if targets.insert(atom, best).is_none() {
                    advertised.push(atom);
                }
            }
            let latin1 = latin1_bytes(&String::from_utf8_lossy(&payloads[best].data));
            payloads.push(MimePayload::new("STRING", latin1));
            let atom = client.atoms.STRING;
            if targets.insert(atom, payloads.len() - 1).is_none() {
                advertised.push(atom);
            }
        }

        let timestamp = client.server_time()?;
        client
            .conn
            .set_selection_owner(client.window, sel, timestamp)
            .map_err(x11err)?;
        let owner = client
            .conn
            .get_selection_owner(sel)
            .map_err(x11err)?
            .reply()
            .map_err(x11err)?
            .owner;
        if owner != client.window {
            return Err(Error::Backend(
                "x11: could not take selection ownership".into(),
            ));
        }
        let max_request = client.conn.maximum_request_bytes();
        let chunk = max_request.saturating_sub(128).clamp(4096, 1024 * 1024);
        Ok(Self {
            client,
            selection: sel,
            payloads,
            targets,
            advertised,
            timestamp,
            incr: HashMap::new(),
            chunk,
        })
    }

    fn serve(mut self) {
        loop {
            let event = match self.client.next_event() {
                Ok(event) => event,
                Err(e) => {
                    debug!(error = %e, "x11 owner connection closed");
                    return;
                }
            };
            if !self.incr.is_empty() {
                self.expire_stalled_transfers();
            }
            match event {
                Event::SelectionClear(e) if e.selection == self.selection => {
                    debug!("x11: selection ownership taken by another client");
                    let _ = self.client.conn.destroy_window(self.client.window);
                    let _ = self.client.conn.flush();
                    return;
                }
                Event::SelectionRequest(req) => {
                    if let Err(e) = self.answer(req) {
                        debug!(error = %e, "x11: selection request failed");
                    }
                }
                Event::PropertyNotify(e) if e.state == Property::DELETE => {
                    if let Err(err) = self.continue_incr(e.window, e.atom) {
                        debug!(error = %err, "x11: INCR transfer failed");
                        self.incr.remove(&(e.window, e.atom));
                        self.stop_watching_if_idle(e.window);
                    }
                }
                _ => {}
            }
        }
    }

    fn answer(&mut self, req: SelectionRequestEvent) -> Result<()> {
        let atoms = &self.client.atoms;
        // Obsolete clients pass None; ICCCM says use the target as property.
        let property = if req.property == x11rb::NONE {
            req.target
        } else {
            req.property
        };
        let mut granted = property;

        if req.target == atoms.TARGETS {
            self.client
                .conn
                .change_property32(
                    PropMode::REPLACE,
                    req.requestor,
                    property,
                    AtomEnum::ATOM,
                    &self.advertised,
                )
                .map_err(x11err)?;
        } else if req.target == atoms.TIMESTAMP {
            self.client
                .conn
                .change_property32(
                    PropMode::REPLACE,
                    req.requestor,
                    property,
                    AtomEnum::INTEGER,
                    &[self.timestamp],
                )
                .map_err(x11err)?;
        } else if let Some(&index) = self.targets.get(&req.target) {
            let data = &self.payloads[index].data;
            if data.len() > INCR_THRESHOLD {
                self.client
                    .conn
                    .change_property32(
                        PropMode::REPLACE,
                        req.requestor,
                        property,
                        atoms.INCR,
                        &[data.len() as u32],
                    )
                    .map_err(x11err)?;
                self.client
                    .conn
                    .change_window_attributes(
                        req.requestor,
                        &ChangeWindowAttributesAux::new().event_mask(EventMask::PROPERTY_CHANGE),
                    )
                    .map_err(x11err)?;
                self.incr.insert(
                    (req.requestor, property),
                    IncrTransfer {
                        target: req.target,
                        payload: index,
                        offset: 0,
                        last_activity: Instant::now(),
                    },
                );
            } else {
                self.client
                    .conn
                    .change_property8(PropMode::REPLACE, req.requestor, property, req.target, data)
                    .map_err(x11err)?;
            }
        } else {
            granted = x11rb::NONE;
        }

        let notify = SelectionNotifyEvent {
            response_type: SELECTION_NOTIFY_EVENT,
            sequence: 0,
            time: req.time,
            requestor: req.requestor,
            selection: req.selection,
            target: req.target,
            property: granted,
        };
        self.client
            .conn
            .send_event(false, req.requestor, EventMask::NO_EVENT, notify)
            .map_err(x11err)?;
        self.client.conn.flush().map_err(x11err)?;
        Ok(())
    }

    /// The requestor deleted the property: send the next INCR chunk.
    fn continue_incr(&mut self, requestor: Window, property: Atom) -> Result<()> {
        let Some(transfer) = self.incr.get_mut(&(requestor, property)) else {
            return Ok(());
        };
        let data = &self.payloads[transfer.payload].data;
        let end = (transfer.offset + self.chunk).min(data.len());
        let chunk = &data[transfer.offset..end];
        self.client
            .conn
            .change_property8(
                PropMode::REPLACE,
                requestor,
                property,
                transfer.target,
                chunk,
            )
            .map_err(x11err)?;
        self.client.conn.flush().map_err(x11err)?;
        if chunk.is_empty() {
            // Zero-length chunk terminates the transfer.
            self.incr.remove(&(requestor, property));
            self.stop_watching_if_idle(requestor);
        } else {
            transfer.offset = end;
            transfer.last_activity = Instant::now();
        }
        Ok(())
    }

    /// Stop receiving PropertyNotify from a requestor once none of its
    /// transfers is in flight (a requestor may pull several targets at once).
    fn stop_watching_if_idle(&self, requestor: Window) {
        if self.incr.keys().any(|(window, _)| *window == requestor) {
            return;
        }
        let _ = self.client.conn.change_window_attributes(
            requestor,
            &ChangeWindowAttributesAux::new().event_mask(EventMask::NO_EVENT),
        );
        let _ = self.client.conn.flush();
    }

    /// Drop transfers whose requestor stopped making progress.
    fn expire_stalled_transfers(&mut self) {
        let now = Instant::now();
        let stalled: Vec<(Window, Atom)> = self
            .incr
            .iter()
            .filter(|(_, t)| now.duration_since(t.last_activity) > INCR_TIMEOUT)
            .map(|(key, _)| *key)
            .collect();
        for key in stalled {
            debug!(requestor = key.0, "x11: abandoning stalled INCR transfer");
            self.incr.remove(&key);
            self.stop_watching_if_idle(key.0);
        }
    }
}

/// Send a paste keystroke to the focused window through XTEST: Ctrl+V, or
/// Ctrl+Shift+V when the focused window is a terminal emulator, where
/// Ctrl+V is a control character.
fn xtest_paste() -> Result<()> {
    let client = Client::new()?;
    let conn = &client.conn;
    let root = client.root;
    conn.xtest_get_version(2, 2)
        .map_err(x11err)?
        .reply()
        .map_err(|_| Error::Backend("x11: XTEST extension unavailable".into()))?;
    let terminal = focused_app(&client).is_some_and(|app| panora_core::apps::is_terminal(&app));
    let control = keycode_for(conn, 0xffe3)
        .ok_or_else(|| Error::Backend("x11: no keycode for Control_L".into()))?;
    let v =
        keycode_for(conn, 0x0076).ok_or_else(|| Error::Backend("x11: no keycode for v".into()))?;
    let shift = if terminal {
        Some(
            keycode_for(conn, 0xffe1)
                .ok_or_else(|| Error::Backend("x11: no keycode for Shift_L".into()))?,
        )
    } else {
        None
    };
    const PRESS: u8 = 2;
    const RELEASE: u8 = 3;
    let mut sequence = vec![(PRESS, control)];
    if let Some(shift) = shift {
        sequence.push((PRESS, shift));
    }
    sequence.push((PRESS, v));
    sequence.push((RELEASE, v));
    if let Some(shift) = shift {
        sequence.push((RELEASE, shift));
    }
    sequence.push((RELEASE, control));
    for (kind, code) in sequence {
        conn.xtest_fake_input(kind, code, x11rb::CURRENT_TIME, root, 0, 0, 0)
            .map_err(x11err)?;
    }
    conn.flush().map_err(x11err)?;
    debug!(terminal, "synthetic paste sent");
    Ok(())
}

/// First keycode producing `keysym` in the current keyboard mapping.
fn keycode_for(conn: &RustConnection, keysym: u32) -> Option<u8> {
    let setup = conn.setup();
    let min = setup.min_keycode;
    let max = setup.max_keycode;
    let count = max.checked_sub(min)? + 1;
    let mapping = conn.get_keyboard_mapping(min, count).ok()?.reply().ok()?;
    let per = usize::from(mapping.keysyms_per_keycode);
    if per == 0 {
        return None;
    }
    mapping
        .keysyms
        .chunks(per)
        .enumerate()
        .find(|(_, syms)| syms.contains(&keysym))
        .map(|(i, _)| min + i as u8)
}

/// Best-effort name of the application the copy came from.
///
/// This is what the privacy engine's `excluded_apps` list matches against, so
/// without it the KeePassXC/Bitwarden/1Password exclusions never fire on X11.
///
/// X11 offers no reliable way to name the *selection owner*: toolkits hand
/// ownership to a hidden proxy window that carries no WM_CLASS. The focused
/// top-level is the same heuristic the GNOME bridge uses and matches what the
/// user was actually working in when they pressed Ctrl+C. Every failure path
/// returns None: a missing source app must never block a capture.
fn focused_app(client: &Client) -> Option<String> {
    let conn = &client.conn;
    let root = client.root;
    let window = active_window(conn, root, client.atoms._NET_ACTIVE_WINDOW)
        .or_else(|| conn.get_input_focus().ok()?.reply().ok().map(|r| r.focus))?;

    // WM_CLASS lives on the top-level, but focus usually sits on a child.
    let mut window = window;
    for _ in 0..WM_CLASS_DEPTH {
        if let Some(class) = wm_class(conn, window) {
            return Some(class);
        }
        let tree = conn.query_tree(window).ok()?.reply().ok()?;
        if tree.parent == x11rb::NONE || tree.parent == window || window == root {
            break;
        }
        window = tree.parent;
    }
    None
}

/// Title of the focused top-level (`_NET_WM_NAME`, else `WM_NAME`), for
/// the `excluded_window_titles` gate. Judged by the daemon and dropped;
/// like the application name, a lookup failure never blocks a capture.
fn focused_title(client: &Client) -> Option<String> {
    let conn = &client.conn;
    let root = client.root;
    let mut window = active_window(conn, root, client.atoms._NET_ACTIVE_WINDOW)
        .or_else(|| conn.get_input_focus().ok()?.reply().ok().map(|r| r.focus))?;
    for _ in 0..WM_CLASS_DEPTH {
        if let Some(title) = window_title(conn, window, &client.atoms) {
            return Some(title);
        }
        let tree = conn.query_tree(window).ok()?.reply().ok()?;
        if tree.parent == x11rb::NONE || tree.parent == window || window == root {
            break;
        }
        window = tree.parent;
    }
    None
}

fn window_title(conn: &RustConnection, window: Window, atoms: &Atoms) -> Option<String> {
    let candidates: [(Atom, Atom); 2] = [
        (atoms._NET_WM_NAME, atoms.UTF8_STRING),
        (AtomEnum::WM_NAME.into(), AtomEnum::STRING.into()),
    ];
    for (property, kind) in candidates {
        let reply = conn
            .get_property(false, window, property, kind, 0, 1024)
            .ok()?
            .reply()
            .ok()?;
        if reply.value.is_empty() {
            continue;
        }
        let title = String::from_utf8_lossy(&reply.value).trim().to_string();
        if !title.is_empty() {
            return Some(title);
        }
    }
    None
}

fn active_window(conn: &RustConnection, root: Window, atom: Atom) -> Option<Window> {
    let reply = conn
        .get_property(false, root, atom, AtomEnum::WINDOW, 0, 1)
        .ok()?
        .reply()
        .ok()?;
    let id = reply.value32()?.next()?;
    (id != x11rb::NONE).then_some(id)
}

/// WM_CLASS is "instance\0class\0"; the class half is the stable app name.
fn wm_class(conn: &RustConnection, window: Window) -> Option<String> {
    let reply = conn
        .get_property(false, window, AtomEnum::WM_CLASS, AtomEnum::STRING, 0, 256)
        .ok()?
        .reply()
        .ok()?;
    if reply.value.is_empty() {
        return None;
    }
    let text = String::from_utf8_lossy(&reply.value);
    let mut parts = text.split('\0').filter(|part| !part.is_empty());
    let instance = parts.next()?;
    Some(parts.next().unwrap_or(instance).to_string())
}
