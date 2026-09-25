// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! The sync node: one QUIC endpoint, the sessions with the other members,
//! pairing in both roles, discovery, and the shared state they all work
//! on. `panora-sync run` builds one; the integration tests build two.

use crate::discovery::{self, Discovery};
use crate::error::{Error, Result};
use crate::group::{GroupKey, GroupState, Member, Roster, RosterUpdate, Sealed};
use crate::identity::{DeviceIdentity, PublicIdentity};
use crate::invite::{Invitation, PairingWindow};
use crate::pairing::{Inviter, JoinRequest, Joiner, PairMessage, SasCode, PROTOCOL};
use crate::panod::Panod;
use crate::session::{self, Command};
use crate::state::SyncState;
use crate::transport::{self, Transport, ALPN_PAIR, ALPN_SYNC};
use crate::wire;
use panora_core::storage::MasterKey;
use panora_core::sync::{SyncCursor, SyncRecord, SyncScope};
use quinn::Connection;
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, RwLock};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot, watch, Notify, OwnedSemaphorePermit, Semaphore};
use tracing::{debug, info, warn};

/// How long a user has to answer a pairing question.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(180);
/// A whole pairing session, answer included, may not take longer.
const PAIRING_TIMEOUT: Duration = Duration::from_secs(240);
/// How often the node looks for peers it is not connected to.
const DIAL_INTERVAL: Duration = Duration::from_secs(15);
/// Dialling one address may not take longer.
const DIAL_TIMEOUT: Duration = Duration::from_secs(10);
/// Each pairing message may take this long to arrive (the user's answer
/// has `ANSWER_TIMEOUT`).
const STEP_TIMEOUT: Duration = Duration::from_secs(30);
/// Connections handled at once, and from one address.
const MAX_CONNECTIONS: usize = 64;
const MAX_CONNECTIONS_PER_ADDRESS: usize = 4;

/// What a node needs to know about its surroundings.
#[derive(Debug, Clone)]
pub struct NodeConfig {
    /// UDP address to listen on.
    pub listen: SocketAddr,
    /// Peers to dial directly (networks without mDNS).
    pub peers: Vec<SocketAddr>,
    /// Advertise and browse with mDNS.
    pub discovery: bool,
    /// `panod`'s socket.
    pub panod_socket: PathBuf,
    /// The sealed state file.
    pub state_path: PathBuf,
    /// This device's `panod` device id.
    pub device_id: String,
    /// What this device shares.
    pub scope: SyncScope,
    /// Turn `sync.enabled` on or off in the user's configuration when this
    /// device joins or leaves a group (off in tests).
    pub manage_config: bool,
}

/// Something that happened in an inviter's pairing window.
#[derive(Debug)]
pub enum PairEvent {
    /// Show this code (code mode), now.
    Code(SasCode),
    /// A device asks to join; answer through `reply`.
    Request {
        /// The code the user compares, in code mode.
        code: Option<SasCode>,
        /// Who is asking.
        request: JoinRequest,
        /// `true` admits it.
        reply: oneshot::Sender<bool>,
    },
    /// The device was admitted; the window is closed.
    Joined(JoinRequest),
    /// An attempt failed; the window may still be open.
    Failed(String),
}

struct PairingSlot {
    window: PairingWindow,
    events: mpsc::UnboundedSender<PairEvent>,
}

struct PeerLink {
    commands: mpsc::UnboundedSender<Command>,
    canonical: bool,
    id: u64,
}

/// A row of [`Node::status`]'s device list.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DeviceRow {
    /// Device name.
    pub name: String,
    /// Identity fingerprint.
    pub fingerprint: String,
    /// This device.
    pub this_device: bool,
    /// A session with it is up.
    pub connected: bool,
}

/// What `panora-sync status` shows.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Status {
    /// This device's name.
    pub device_name: String,
    /// This device's identity fingerprint.
    pub fingerprint: String,
    /// Where the node listens.
    pub listen: String,
    /// Whether a group exists at all.
    pub in_group: bool,
    /// Whether this device is a member of it right now.
    pub member: bool,
    /// Whether this device holds the current group key.
    pub has_key: bool,
    /// Roster epoch.
    pub epoch: u64,
    /// The device list.
    pub devices: Vec<DeviceRow>,
}

/// State shared by every task of a node.
pub(crate) struct Shared {
    pub(crate) config: NodeConfig,
    pub(crate) panod: Panod,
    identity: RwLock<Arc<DeviceIdentity>>,
    state_key: MasterKey,
    state: Mutex<SyncState>,
    transport: Transport,
    discovery: Option<Discovery>,
    peers: Mutex<HashMap<PublicIdentity, PeerLink>>,
    dialling: Mutex<HashSet<SocketAddr>>,
    addr_of: Mutex<HashMap<SocketAddr, PublicIdentity>>,
    pairing: tokio::sync::Mutex<Option<PairingSlot>>,
    /// Bumped whenever a pairing window opens or closes; a pairing that
    /// started under another value is cancelled before it admits anyone.
    window_epoch: AtomicU64,
    window_changed: Notify,
    connection_slots: Arc<Semaphore>,
    per_address: Mutex<HashMap<IpAddr, usize>>,
    changes: watch::Receiver<u64>,
    dial_now: Notify,
    next_link: AtomicU64,
    shutdown: watch::Sender<bool>,
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// One accepted connection's claim on the connection limits.
struct ConnectionSlot {
    shared: Arc<Shared>,
    ip: IpAddr,
    _permit: OwnedSemaphorePermit,
}

impl Drop for ConnectionSlot {
    fn drop(&mut self) {
        let mut counts = self
            .shared
            .per_address
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(n) = counts.get_mut(&self.ip) {
            *n -= 1;
            if *n == 0 {
                counts.remove(&self.ip);
            }
        }
    }
}

impl Shared {
    /// This device's current identity (a new one after `leave`).
    pub(crate) fn identity(&self) -> Arc<DeviceIdentity> {
        self.identity
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    fn claim_connection(self: &Arc<Self>, ip: IpAddr) -> Option<ConnectionSlot> {
        let permit = self.connection_slots.clone().try_acquire_owned().ok()?;
        let mut counts = self.per_address.lock().unwrap_or_else(|e| e.into_inner());
        let n = counts.entry(ip).or_insert(0);
        if *n >= MAX_CONNECTIONS_PER_ADDRESS {
            return None;
        }
        *n += 1;
        Some(ConnectionSlot {
            shared: self.clone(),
            ip,
            _permit: permit,
        })
    }

    fn lock(&self) -> MutexGuard<'_, SyncState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn save(&self, state: &SyncState) {
        if let Err(e) = state.save(&self.config.state_path, &self.state_key) {
            warn!(error = %e, "could not save the sync state");
        }
    }

    pub(crate) fn changes(&self) -> watch::Receiver<u64> {
        self.changes.clone()
    }

    pub(crate) fn roster_chain(&self) -> Option<Vec<Roster>> {
        self.lock().group.as_ref().map(GroupState::welcome_chain)
    }

    pub(crate) fn is_peer(&self, peer: &PublicIdentity) -> bool {
        self.lock()
            .group
            .as_ref()
            .is_some_and(|g| g.is_member() && g.is_peer(peer))
    }

    pub(crate) fn device_id_of(&self, peer: &PublicIdentity) -> Option<String> {
        let state = self.lock();
        let member = state
            .group
            .as_ref()?
            .current()
            .member(peer)?
            .device_id
            .clone();
        Some(member)
    }

    pub(crate) fn has_current_key(&self) -> bool {
        self.lock()
            .group
            .as_ref()
            .is_some_and(|g| g.current_key().is_some())
    }

    pub(crate) fn key_for(&self, peer: &PublicIdentity) -> Option<GroupKey> {
        self.lock().group.as_ref()?.key_for_peer(peer).ok()
    }

    pub(crate) fn cursor(&self, peer: &PublicIdentity) -> SyncCursor {
        self.lock().cursors.get(peer).copied().unwrap_or_default()
    }

    pub(crate) fn set_cursor(&self, peer: PublicIdentity, cursor: SyncCursor) {
        let mut state = self.lock();
        state.cursors.insert(peer, cursor);
        self.save(&state);
    }

    pub(crate) fn seal_records(&self, records: &[SyncRecord]) -> Result<Vec<Sealed>> {
        let state = self.lock();
        let group = state.group.as_ref().ok_or(Error::Roster("no group"))?;
        records.iter().map(|r| group.seal_record(r)).collect()
    }

    pub(crate) fn open_records(&self, sealed: &[Sealed]) -> Result<Vec<SyncRecord>> {
        let state = self.lock();
        let group = state.group.as_ref().ok_or(Error::Roster("no group"))?;
        sealed.iter().map(|s| group.open_record(s)).collect()
    }

    pub(crate) fn receive_key(&self, key: GroupKey) {
        let accepted = {
            let mut state = self.lock();
            let Some(group) = state.group.as_mut() else {
                return;
            };
            match group.accept_key(key) {
                Ok(new) => {
                    if new {
                        self.save(&state);
                    }
                    new
                }
                Err(e) => {
                    debug!(error = %e, "ignored a key share");
                    false
                }
            }
        };
        if accepted {
            info!("received the current group key");
            self.broadcast(|| Command::Nudge);
        }
    }

    /// Apply a peer's rosters and do whatever follows: redo this device's
    /// changes after a lost fork, tell every session, drop sessions with
    /// devices that left, stop if this device was removed.
    pub(crate) fn receive_rosters(&self, from: PublicIdentity, rosters: Vec<Roster>) {
        let outcome = {
            let mut state = self.lock();
            let Some(group) = state.group.as_mut() else {
                return;
            };
            let outcome = match group.apply_chain(rosters) {
                Ok(RosterUpdate::Unchanged) => Outcome::Nothing,
                Ok(RosterUpdate::ForkLost) => Outcome::TellSender,
                Ok(RosterUpdate::Advanced) => Outcome::Changed(Vec::new()),
                Ok(RosterUpdate::Replaced { reapply }) => {
                    match group.reapply(&self.identity(), reapply, now()) {
                        Ok(redone) => {
                            Outcome::Changed(redone.into_iter().filter_map(|(_, k)| k).collect())
                        }
                        Err(e) => {
                            warn!(error = %e, "could not redo changes after a fork");
                            Outcome::Changed(Vec::new())
                        }
                    }
                }
                Ok(RosterUpdate::RemovedThisDevice) => Outcome::Removed,
                Err(e) => {
                    debug!(error = %e, "could not apply a peer's rosters");
                    Outcome::TellSender
                }
            };
            if !matches!(outcome, Outcome::Nothing | Outcome::TellSender) {
                self.save(&state);
            }
            outcome
        };
        match outcome {
            Outcome::Nothing => {}
            Outcome::TellSender => {
                // Only a member learns the current device list; a removed
                // device replaying its old chain does not.
                if self.is_peer(&from) {
                    if let Some(chain) = self.roster_chain() {
                        self.send_to(&from, Command::Rosters(chain));
                    }
                }
            }
            Outcome::Changed(new_keys) => {
                info!("the device list changed");
                self.after_roster_change(new_keys);
            }
            Outcome::Removed => {
                warn!("this device was removed from the sync group");
                self.broadcast(|| Command::Close);
                self.set_sync_enabled(false);
            }
        }
    }

    /// Tell every session about the current rosters, hand new keys to the
    /// members connected now, and close sessions with non-members.
    fn after_roster_change(&self, new_keys: Vec<GroupKey>) {
        if let Some(chain) = self.roster_chain() {
            self.broadcast(|| Command::Rosters(chain.clone()));
        }
        let peers: Vec<PublicIdentity> = self.lock_peers().keys().copied().collect();
        for peer in peers {
            if !self.is_peer(&peer) {
                self.send_to(&peer, Command::Close);
                continue;
            }
            if !new_keys.is_empty() {
                if let Some(key) = self.key_for(&peer) {
                    self.send_to(&peer, Command::KeyShare(key));
                }
            }
        }
        self.dial_now.notify_one();
    }

    fn lock_peers(&self) -> MutexGuard<'_, HashMap<PublicIdentity, PeerLink>> {
        self.peers.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn broadcast(&self, command: impl Fn() -> Command) {
        for link in self.lock_peers().values() {
            let _ = link.commands.send(command());
        }
    }

    fn send_to(&self, peer: &PublicIdentity, command: Command) {
        if let Some(link) = self.lock_peers().get(peer) {
            let _ = link.commands.send(command);
        }
    }

    fn set_sync_enabled(&self, enabled: bool) {
        if !self.config.manage_config {
            return;
        }
        let result = panora_core::config::Config::load().and_then(|mut config| {
            if config.sync.enabled == enabled {
                return Ok(false);
            }
            config.sync.enabled = enabled;
            config.save().map(|_| true)
        });
        match result {
            Ok(true) => {
                let panod = self.panod.clone();
                tokio::spawn(async move {
                    if let Err(e) = panod.reload_config().await {
                        warn!(error = %e, "panod did not reload its configuration");
                    }
                });
            }
            Ok(false) => {}
            Err(e) => warn!(error = %e, "could not update sync.enabled in the configuration"),
        }
    }

    /// Register a session; `None` if one with that peer already runs and
    /// wins. Of two sessions with the same peer, the one the lower
    /// identity dialled is kept, on both sides.
    fn register(
        &self,
        peer: PublicIdentity,
        initiator: bool,
    ) -> Option<(u64, mpsc::UnboundedReceiver<Command>)> {
        let canonical = initiator == (self.identity().public() < peer);
        let mut peers = self.lock_peers();
        if let Some(existing) = peers.get(&peer) {
            if existing.canonical || !canonical {
                return None;
            }
            let _ = existing.commands.send(Command::Close);
        }
        let (tx, rx) = mpsc::unbounded_channel();
        let id = self.next_link.fetch_add(1, Ordering::Relaxed);
        peers.insert(
            peer,
            PeerLink {
                commands: tx,
                canonical,
                id,
            },
        );
        Some((id, rx))
    }

    fn unregister(&self, peer: &PublicIdentity, id: u64) {
        let mut peers = self.lock_peers();
        if peers.get(peer).is_some_and(|l| l.id == id) {
            peers.remove(peer);
        }
    }
}

enum Outcome {
    Nothing,
    TellSender,
    Changed(Vec<GroupKey>),
    Removed,
}

/// A running sync node.
#[derive(Clone)]
pub struct Node {
    shared: Arc<Shared>,
}

impl Node {
    /// Bind the endpoint and start listening, dialling and (if configured)
    /// discovery. Must run inside a tokio runtime.
    pub async fn start(config: NodeConfig, state: SyncState, state_key: MasterKey) -> Result<Self> {
        let transport = Transport::bind(config.listen)?;
        let identity = DeviceIdentity::from_pkcs8(state.identity.pkcs8())?;
        let panod = Panod::new(config.panod_socket.clone());
        let changes = panod.watch();
        let discovery = if config.discovery {
            match Discovery::start(transport.local_addr()?.port()) {
                Ok(d) => Some(d),
                Err(e) => {
                    warn!(error = %e, "mDNS unavailable; only configured peers will be reached");
                    None
                }
            }
        } else {
            None
        };
        let (shutdown, _) = watch::channel(false);
        let shared = Arc::new(Shared {
            config,
            panod,
            identity: RwLock::new(Arc::new(identity)),
            state_key,
            state: Mutex::new(state),
            transport,
            discovery,
            peers: Mutex::new(HashMap::new()),
            dialling: Mutex::new(HashSet::new()),
            addr_of: Mutex::new(HashMap::new()),
            pairing: tokio::sync::Mutex::new(None),
            window_epoch: AtomicU64::new(0),
            window_changed: Notify::new(),
            connection_slots: Arc::new(Semaphore::new(MAX_CONNECTIONS)),
            per_address: Mutex::new(HashMap::new()),
            changes,
            dial_now: Notify::new(),
            next_link: AtomicU64::new(1),
            shutdown,
        });
        let node = Self { shared };
        node.advertise();
        tokio::spawn(accept_loop(node.shared.clone()));
        tokio::spawn(dial_loop(node.shared.clone()));
        info!(
            listen = %node.local_addr(),
            fingerprint = %node.shared.identity().public().fingerprint(),
            "sync node started"
        );
        Ok(node)
    }

    /// Where the node listens.
    pub fn local_addr(&self) -> SocketAddr {
        self.shared
            .transport
            .local_addr()
            .unwrap_or_else(|_| self.shared.config.listen)
    }

    /// This device's identity.
    pub fn identity(&self) -> PublicIdentity {
        self.shared.identity().public()
    }

    fn advertise(&self) {
        if let Some(discovery) = &self.shared.discovery {
            let group = self.shared.lock().group.as_ref().map(|g| g.group_id());
            discovery.advertise_sync(group.as_ref());
        }
    }

    /// Status and device list.
    pub fn status(&self) -> Status {
        let connected: HashSet<PublicIdentity> = self.shared.lock_peers().keys().copied().collect();
        let state = self.shared.lock();
        let group = state.group.as_ref();
        Status {
            device_name: state.device_name.clone(),
            fingerprint: self.shared.identity().public().fingerprint(),
            listen: self.local_addr().to_string(),
            in_group: group.is_some(),
            member: group.is_some_and(GroupState::is_member),
            has_key: group.is_some_and(|g| g.current_key().is_some()),
            epoch: group.map(|g| g.current().epoch).unwrap_or(0),
            devices: group
                .map(|g| {
                    g.devices()
                        .into_iter()
                        .map(|d| DeviceRow {
                            connected: connected.contains(&d.member.identity),
                            name: d.member.name,
                            fingerprint: d.fingerprint,
                            this_device: d.this_device,
                        })
                        .collect()
                })
                .unwrap_or_default(),
        }
    }

    fn ensure_group(&self) -> Result<()> {
        let mut state = self.shared.lock();
        match &state.group {
            Some(g) if g.is_member() => Ok(()),
            Some(_) => Err(Error::Roster(
                "this device is not a member of its group; leave it first",
            )),
            None => {
                let group = GroupState::create(
                    &self.shared.identity(),
                    &self.shared.config.device_id,
                    &state.device_name.clone(),
                    now(),
                )?;
                state.group = Some(group);
                self.shared.save(&state);
                drop(state);
                self.shared.set_sync_enabled(true);
                self.advertise();
                info!("created a new sync group");
                Ok(())
            }
        }
    }

    async fn open_window(
        &self,
        window: PairingWindow,
    ) -> (u64, mpsc::UnboundedReceiver<PairEvent>) {
        let (events, rx) = mpsc::unbounded_channel();
        // A running pairing holds the slot for at most one step's timeout.
        let epoch = self.shared.bump_window();
        *self.shared.pairing.lock().await = Some(PairingSlot { window, events });
        if let Some(d) = &self.shared.discovery {
            d.advertise_pair(
                &self.shared.identity().public(),
                &self.shared.lock().device_name,
            );
        }
        (epoch, rx)
    }

    /// Start a group if there is none, and accept the device holding the
    /// returned invitation. Its link is what the QR code shows. The number
    /// identifies this window for [`Self::close_window`].
    pub async fn invite(&self) -> Result<(Invitation, mpsc::UnboundedReceiver<PairEvent>, u64)> {
        self.ensure_group()?;
        let port = self.local_addr().port();
        let addrs = discovery::local_addresses(port);
        let invitation = Invitation::new(self.shared.identity().public(), addrs, now());
        let (epoch, events) = self
            .open_window(PairingWindow::for_invitation(invitation.clone()))
            .await;
        Ok((invitation, events, epoch))
    }

    /// Start a group if there is none, and accept one device whose user
    /// compares a code with this one.
    pub async fn open_code_window(&self) -> Result<(mpsc::UnboundedReceiver<PairEvent>, u64)> {
        self.ensure_group()?;
        let (epoch, events) = self.open_window(PairingWindow::for_code(now())).await;
        Ok((events, epoch))
    }

    /// Stop accepting devices through the window `epoch` opened (a newer
    /// window stays open). A pairing in progress in it is cancelled before
    /// it admits anyone.
    pub async fn close_window(&self, epoch: u64) {
        if self.shared.window_epoch.load(Ordering::SeqCst) != epoch {
            return;
        }
        self.shared.bump_window();
        if let Ok(mut slot) = self.shared.pairing.try_lock() {
            *slot = None;
        }
        if let Some(d) = &self.shared.discovery {
            d.stop_pair();
        }
    }

    fn check_can_join(&self) -> Result<()> {
        let state = self.shared.lock();
        if state.group.as_ref().is_some_and(|g| g.is_member()) {
            return Err(Error::Roster(
                "this device is already in a group; leave it first",
            ));
        }
        Ok(())
    }

    /// Join the group of the device that made `invitation`.
    pub async fn join(&self, invitation: Invitation) -> Result<()> {
        self.check_can_join()?;
        let mut targets = invitation.addrs.clone();
        if let Some(d) = &self.shared.discovery {
            for found in d.find_pair(Duration::from_secs(4)).await {
                if found.tag == discovery::identity_tag(&invitation.inviter) {
                    targets.push(found.addr);
                }
            }
        }
        if targets.is_empty() {
            return Err(Error::Invitation(
                "the inviting device could not be found on this network",
            ));
        }
        let mut last = Error::Invitation("the inviting device could not be reached");
        for addr in targets {
            match self
                .join_at(addr, Some(invitation.clone()), |_, _| async { true })
                .await
            {
                Ok(()) => return Ok(()),
                Err(e @ (Error::Transport(_) | Error::Io(_))) => last = e,
                Err(e) => return Err(e),
            }
        }
        Err(last)
    }

    /// Join by comparing a code with a device that opened a code window:
    /// at `addr`, or the only one mDNS finds. `confirm` shows the code and
    /// returns the user's answer.
    pub async fn join_by_code<F, Fut>(&self, addr: Option<SocketAddr>, confirm: F) -> Result<()>
    where
        F: FnOnce(SasCode, PublicIdentity) -> Fut,
        Fut: Future<Output = bool>,
    {
        self.check_can_join()?;
        let addr = match addr {
            Some(addr) => addr,
            None => {
                let found = match &self.shared.discovery {
                    Some(d) => d.find_pair(Duration::from_secs(4)).await,
                    None => Vec::new(),
                };
                match found.as_slice() {
                    [one] => one.addr,
                    [] => {
                        return Err(Error::Invitation(
                            "no device on this network is waiting for a code; give its address",
                        ))
                    }
                    _ => {
                        return Err(Error::Invitation(
                            "several devices are waiting for a code; give the address of one",
                        ))
                    }
                }
            }
        };
        self.join_at(addr, None, |code, inviter| async move {
            match code {
                Some(code) => confirm(code, inviter).await,
                None => false,
            }
        })
        .await
    }

    async fn join_at<F, Fut>(
        &self,
        addr: SocketAddr,
        invitation: Option<Invitation>,
        confirm: F,
    ) -> Result<()>
    where
        F: FnOnce(Option<SasCode>, PublicIdentity) -> Fut,
        Fut: Future<Output = bool>,
    {
        let shared = &self.shared;
        let conn = tokio::time::timeout(DIAL_TIMEOUT, shared.transport.connect(addr, ALPN_PAIR))
            .await
            .map_err(|_| Error::Transport(format!("{addr} did not answer")))??;
        let (send, recv) = tokio::time::timeout(STEP_TIMEOUT, conn.open_bi())
            .await
            .map_err(|_| Error::Transport(format!("{addr} did not answer")))?
            .map_err(|e| Error::Transport(e.to_string()))?;
        let mut stream = tokio::io::join(recv, send);
        let name = shared.lock().device_name.clone();
        let identity = shared.identity();
        let (joiner, commit) = Joiner::start(
            &identity,
            &shared.config.device_id,
            &name,
            invitation,
            now(),
        )?;
        let group = tokio::time::timeout(
            PAIRING_TIMEOUT,
            wire::run_joiner(&mut stream, joiner, commit, confirm),
        )
        .await
        .map_err(|_| Error::Protocol("pairing took too long"))??;
        conn.close(0u32.into(), b"paired");
        {
            let mut state = shared.lock();
            state.group = Some(group);
            state.cursors.clear();
            shared.save(&state);
        }
        info!("joined a sync group");
        shared.set_sync_enabled(true);
        self.advertise();
        // The inviter is reachable where pairing just worked.
        shared.dial(addr);
        Ok(())
    }

    /// Remove the device whose fingerprint starts with, or whose name is,
    /// `device`. The key moves on and every connected member gets it.
    pub fn remove(&self, device: &str) -> Result<Member> {
        let (member, key) = {
            let mut state = self.shared.lock();
            let group = state.group.as_mut().ok_or(Error::Roster("no group"))?;
            let wanted = device.trim();
            let matches: Vec<Member> = group
                .current()
                .members
                .iter()
                .filter(|m| {
                    m.name == wanted
                        || (wanted.len() >= 4
                            && m.identity.fingerprint().starts_with(&wanted.to_lowercase()))
                })
                .cloned()
                .collect();
            let member = match matches.as_slice() {
                [one] => one.clone(),
                [] => return Err(Error::Device("no device in the group matches that")),
                _ => {
                    return Err(Error::Device(
                        "several devices match; use more of the fingerprint",
                    ))
                }
            };
            let (_, key) = group.remove_member(&self.shared.identity(), &member.identity, now())?;
            self.shared.save(&state);
            (member, key)
        };
        info!(device = %member.name, "removed a device from the group");
        // The removed device gets the new roster first, then its session
        // closes (after_roster_change does both, in that order).
        self.shared.after_roster_change(vec![key]);
        Ok(member)
    }

    /// Leave the group on this device: forget it, take a new identity (the
    /// other devices should still remove this one), stop syncing.
    pub fn leave(&self) -> Result<()> {
        {
            let mut state = self.shared.lock();
            let fresh = DeviceIdentity::generate()?;
            let copy = DeviceIdentity::from_pkcs8(fresh.pkcs8())?;
            state.group = None;
            state.cursors.clear();
            state.identity = fresh;
            self.shared.save(&state);
            // Everything from here on (sessions, pairing, a new group) uses
            // the new identity, the same one the state file now holds.
            *self
                .shared
                .identity
                .write()
                .unwrap_or_else(|e| e.into_inner()) = Arc::new(copy);
        }
        self.advertise();
        self.shared.broadcast(|| Command::Close);
        self.shared.set_sync_enabled(false);
        Ok(())
    }

    /// Stop everything.
    pub fn shutdown(&self) {
        let _ = self.shared.shutdown.send(true);
        self.shared.broadcast(|| Command::Close);
        self.shared.transport.close();
        if let Some(d) = &self.shared.discovery {
            d.shutdown();
        }
    }

    /// Dial `addr` now (tests, and after pairing).
    pub fn dial(&self, addr: SocketAddr) {
        self.shared.dial(addr);
    }

    /// Identities with a running session.
    pub fn connected(&self) -> Vec<PublicIdentity> {
        self.shared.lock_peers().keys().copied().collect()
    }
}

impl Shared {
    fn dial(self: &Arc<Self>, addr: SocketAddr) {
        {
            let mut dialling = self.dialling.lock().unwrap_or_else(|e| e.into_inner());
            if !dialling.insert(addr) {
                return;
            }
        }
        let shared = self.clone();
        tokio::spawn(async move {
            let result = shared.dial_once(addr).await;
            shared
                .dialling
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&addr);
            if let Err(e) = result {
                debug!(%addr, error = %e, "sync dial failed");
            }
        });
    }

    async fn dial_once(self: &Arc<Self>, addr: SocketAddr) -> Result<()> {
        let conn = tokio::time::timeout(DIAL_TIMEOUT, self.transport.connect(addr, ALPN_SYNC))
            .await
            .map_err(|_| Error::Transport(format!("{addr} did not answer")))??;
        let (send, recv, peer) = transport::open_authenticated(&conn, &self.identity()).await?;
        self.addr_of
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(addr, peer);
        self.clone().run_session(conn, send, recv, peer, true).await
    }

    async fn run_session(
        self: Arc<Self>,
        conn: Connection,
        send: quinn::SendStream,
        recv: quinn::RecvStream,
        peer: PublicIdentity,
        initiator: bool,
    ) -> Result<()> {
        let Some((id, commands)) = self.register(peer, initiator) else {
            conn.close(0u32.into(), b"duplicate");
            return Ok(());
        };
        info!(peer = %peer.fingerprint(), "sync session up");
        let result = session::run(self.clone(), conn.clone(), send, recv, peer, commands).await;
        self.unregister(&peer, id);
        // Let what was last written (a roster, a goodbye) reach the peer
        // before tearing the connection down.
        let _ = tokio::time::timeout(Duration::from_secs(2), conn.closed()).await;
        conn.close(0u32.into(), b"done");
        info!(peer = %peer.fingerprint(), "sync session down");
        result
    }

    fn bump_window(&self) -> u64 {
        let epoch = self.window_epoch.fetch_add(1, Ordering::SeqCst) + 1;
        self.window_changed.notify_waiters();
        epoch
    }

    /// One pairing session as the inviter, if a window is open.
    ///
    /// Nothing is spent before the joiner's first message has arrived and
    /// is a well-formed commitment: a stranger opening connections cannot
    /// use up the window's attempts or hold its lock by saying nothing.
    async fn answer_pairing(self: &Arc<Self>, conn: Connection) -> Result<()> {
        let (send, recv) = tokio::time::timeout(STEP_TIMEOUT, conn.accept_bi())
            .await
            .map_err(|_| Error::Protocol("the other device said nothing"))?
            .map_err(|e| Error::Transport(e.to_string()))?;
        let mut stream = tokio::io::join(recv, send);
        let commit = tokio::time::timeout(STEP_TIMEOUT, wire::read_message(&mut stream))
            .await
            .map_err(|_| Error::Protocol("the other device said nothing"))??;
        if !matches!(&commit, PairMessage::Commit { protocol, .. } if protocol == PROTOCOL) {
            return Err(Error::Protocol("not a pairing request"));
        }
        let epoch = self.window_epoch.load(Ordering::SeqCst);
        let mut slot_guard = tokio::time::timeout(STEP_TIMEOUT, self.pairing.lock())
            .await
            .map_err(|_| Error::Protocol("another pairing is in progress"))?;
        let Some(slot) = slot_guard.as_mut() else {
            wire::write_message(
                &mut stream,
                &PairMessage::Abort {
                    reason: crate::pairing::AbortReason::Closed,
                },
            )
            .await?;
            return Ok(());
        };
        let events = slot.events.clone();
        let result = tokio::time::timeout(
            PAIRING_TIMEOUT,
            self.run_inviter(&mut stream, commit, &mut slot.window, &events, epoch),
        )
        .await
        .unwrap_or(Err(Error::Protocol("pairing took too long")));
        let finished = result.is_ok()
            || !slot.window.is_open(now())
            || self.window_epoch.load(Ordering::SeqCst) != epoch;
        match &result {
            Ok(request) => {
                let _ = events.send(PairEvent::Joined(request.clone()));
            }
            Err(e) => {
                let _ = events.send(PairEvent::Failed(e.to_string()));
            }
        }
        if finished {
            *slot_guard = None;
            if let Some(d) = &self.discovery {
                d.stop_pair();
            }
        }
        result.map(|_| ())
    }

    async fn run_inviter<S>(
        self: &Arc<Self>,
        stream: &mut S,
        commit: PairMessage,
        window: &mut PairingWindow,
        events: &mpsc::UnboundedSender<PairEvent>,
        epoch: u64,
    ) -> Result<JoinRequest>
    where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    {
        let identity = self.identity();
        let mut inviter = {
            let state = self.lock();
            let group = state.group.as_ref().ok_or(Error::Roster("no group"))?;
            Inviter::new(&identity, group, window, now())?
        };
        let result = async {
            let offer = inviter.on_commit(commit)?;
            wire::write_message(stream, &offer).await?;
            inviter.on_reveal(read_step(stream).await?)?;
            if let Some(code) = inviter.code() {
                let _ = events.send(PairEvent::Code(code));
            }
            let request = inviter.on_join(read_step(stream).await?)?;
            let code = inviter.code();
            let approved = match code {
                // Invitation mode: holding the link is the authorisation.
                None => true,
                Some(_) => {
                    let (reply, answer) = oneshot::channel();
                    let _ = events.send(PairEvent::Request {
                        code,
                        request: request.clone(),
                        reply,
                    });
                    let closed = self.window_changed.notified();
                    tokio::select! {
                        answer = tokio::time::timeout(ANSWER_TIMEOUT, answer) => {
                            matches!(answer, Ok(Ok(true)))
                        }
                        _ = closed => false,
                    }
                }
            };
            if !approved || self.window_epoch.load(Ordering::SeqCst) != epoch {
                wire::write_message(stream, &inviter.reject()).await?;
                return Err(Error::Cancelled);
            }
            // Admit on a copy and keep it only once the welcome is out, so
            // a joiner that vanished now is not left in the device list.
            let (welcome, roster, admitted) = {
                let state = self.lock();
                let mut group = state.group.clone().ok_or(Error::Roster("no group"))?;
                let (welcome, roster) = inviter.approve(&mut group, now())?;
                (welcome, roster, group)
            };
            wire::write_message(stream, &welcome).await?;
            {
                let mut state = self.lock();
                let Some(group) = state.group.as_mut() else {
                    return Err(Error::Roster("the group went away during pairing"));
                };
                if group.current().hash() == roster.prev {
                    *group = admitted;
                } else {
                    // The device list changed meanwhile: add the new
                    // roster on top the ordinary way.
                    group.apply_roster(roster)?;
                }
                self.save(&state);
            }
            Ok(request)
        }
        .await;
        if let Err(e) = &result {
            if let Some(reason) = wire::abort_reason(e) {
                let _ = wire::write_message(stream, &PairMessage::Abort { reason }).await;
            }
        }
        if result.is_ok() {
            info!("a device joined the group");
            self.after_roster_change(Vec::new());
        }
        result
    }
}

/// The next pairing message, within one step's time.
async fn read_step<S: tokio::io::AsyncRead + Unpin>(stream: &mut S) -> Result<PairMessage> {
    tokio::time::timeout(STEP_TIMEOUT, wire::read_message(stream))
        .await
        .map_err(|_| Error::Protocol("the other device stopped answering"))?
}

async fn accept_loop(shared: Arc<Shared>) {
    while let Some(incoming) = shared.transport.accept().await {
        let Some(slot) = shared.claim_connection(incoming.remote_address().ip()) else {
            incoming.refuse();
            continue;
        };
        let shared = shared.clone();
        tokio::spawn(async move {
            let _slot = slot;
            let conn = match incoming.await {
                Ok(conn) => conn,
                Err(e) => {
                    debug!(error = %e, "incoming handshake failed");
                    return;
                }
            };
            let result = match transport::alpn(&conn).as_deref() {
                Some(ALPN_SYNC) => {
                    match transport::accept_authenticated(&conn, &shared.identity()).await {
                        Ok((send, recv, peer)) => {
                            shared
                                .clone()
                                .run_session(conn, send, recv, peer, false)
                                .await
                        }
                        Err(e) => Err(e),
                    }
                }
                Some(ALPN_PAIR) => {
                    let result = shared.answer_pairing(conn.clone()).await;
                    // Dropping the connection now would discard the last
                    // message before the joiner reads it; the joiner closes
                    // once it has.
                    let _ = tokio::time::timeout(Duration::from_secs(10), conn.closed()).await;
                    result
                }
                _ => Err(Error::Protocol("unknown protocol")),
            };
            if let Err(e) = result {
                debug!(error = %e, "incoming session ended with an error");
            }
        });
    }
}

async fn dial_loop(shared: Arc<Shared>) {
    let mut shutdown = shared.shutdown.subscribe();
    loop {
        if let Some(d) = &shared.discovery {
            d.refresh();
        }
        let ready = shared.lock().group.as_ref().is_some_and(|g| g.is_member());
        if ready {
            let connected: HashSet<PublicIdentity> = shared.lock_peers().keys().copied().collect();
            let addr_of = shared
                .addr_of
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone();
            let mut targets: Vec<SocketAddr> = shared.config.peers.clone();
            if let Some(d) = &shared.discovery {
                let group = shared.lock().group.as_ref().map(|g| g.group_id());
                if let Some(group) = group {
                    targets.extend(d.sync_peers(&group));
                }
            }
            for addr in targets {
                let known = addr_of.get(&addr);
                // An address once answered by someone else (a spoof, a
                // reused lease) is still worth trying again.
                if known.is_some_and(|id| connected.contains(id)) {
                    continue;
                }
                shared.dial(addr);
            }
        }
        tokio::select! {
            _ = tokio::time::sleep(DIAL_INTERVAL) => {}
            _ = shared.dial_now.notified() => {}
            _ = shutdown.changed() => return,
        }
    }
}
