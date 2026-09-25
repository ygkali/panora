// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! Daemon core: wires backend events through the privacy engine into
//! encrypted storage, and notifies the sync provider (ADR 0002).

use panora_core::backend::{ClipboardBackend, ClipboardEvent, EventKind};
use panora_core::config::Config;
use panora_core::error::{Error, Result};
use panora_core::ipc::v3::MAX_FDS_PER_FRAME;
use panora_core::ipc::{health, CapabilityData, HealthItem, StatusData, PROTOCOL_VERSION};
use panora_core::model::{ClipboardData, ContentKind, Entry, MimePayload, Selection};
use panora_core::privacy::{ContentFilters, PrivacyEngine};
use panora_core::storage::{BlobStore, Database, QueryFilter};
use panora_core::sync::{
    lww_wins, payload_hash, SyncCursor, SyncEvent, SyncProvider, SyncRecord, SyncScope,
};
use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

/// MIME types we always try to capture, in preference order.
pub const WANTED_MIMES: &[&str] = &[
    "text/plain;charset=utf-8",
    "text/plain",
    "UTF8_STRING",
    "text/html",
    "text/rtf",
    "application/rtf",
    "text/uri-list",
    "x-special/gnome-copied-files",
    "application/vnd.kde.cutsel",
    "text/x-color",
    "application/x-color",
    "image/png",
    "image/jpeg",
    "image/webp",
    "image/bmp",
    "image/tiff",
    "image/gif",
    "image/svg+xml",
    "image/x-icon",
    "image/avif",
    "image/heic",
    "image/heif",
];

/// Maximum preview length stored in the database.
const PREVIEW_LEN: usize = 500;
/// What a masked preview starts with (`privacy.sensitive_policy = "mask"`).
pub const SENSITIVE_MASK: &str = "••••••••";

/// Delay between putting data on the clipboard and synthesizing Ctrl+V, so
/// the popup has time to close and focus returns to the target window.
const PASTE_DELAY: std::time::Duration = std::time::Duration::from_millis(160);

/// How long after a recall the GNOME bridge's echo of it is recognised.
const RECALL_ECHO_WINDOW: std::time::Duration = std::time::Duration::from_secs(5);

/// Outcome of a recall request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecallOutcome {
    /// The paste keystroke was delivered.
    pub pasted: bool,
}

/// The daemon: owns backend, privacy engine, storage and sync provider.
pub struct Daemon {
    backend: Arc<dyn ClipboardBackend>,
    privacy: RwLock<PrivacyEngine>,
    db: Database,
    blobs: BlobStore,
    sync: Arc<dyn SyncProvider>,
    config: RwLock<Config>,
    device_id: String,
    revision: AtomicU64,
    /// Mirrors `revision`, but lets `Subscribe` connections `.changed()
    /// .await` instead of polling the atomic. Sent to on every `bump()`.
    revision_watch: tokio::sync::watch::Sender<u64>,
    /// Entry stored from the most recent change of each selection; the only
    /// content the persistence path may re-offer.
    last_stored: Mutex<HashMap<Selection, i64>>,
    /// Entry most recently put on the clipboard by `recall`, with the time.
    /// The GNOME bridge reports our own recall back as a change; matching
    /// the pushed bytes against this entry's blobs keeps it from becoming a
    /// second, single-format copy.
    last_recall: Mutex<Option<(i64, std::time::Instant)>>,
    /// Bumped by `apply_config`; the capture loop subscribes so a changed
    /// `record_primary` opens or drops the PRIMARY watch without a restart.
    config_epoch: tokio::sync::watch::Sender<u64>,
    /// The session is locked (screensaver active). Nothing copied on the
    /// lock screen is recorded; unlike private mode this is not user state.
    locked: std::sync::atomic::AtomicBool,
    /// The GNOME Shell extension owns its bus name (only meaningful when the
    /// backend needs the bridge). Assumed true until the watcher reports, so
    /// a slow bus never produces a false warning at startup.
    extension_present: std::sync::atomic::AtomicBool,
    /// Second-layer password lock (SEC-02). Unlike `locked` (the OS session
    /// lock, which only pauses capture) this gates `List`/`Preview`/
    /// `Recall` in `handle_request`; the master key stays loaded either
    /// way (see `crate::keyring` and `panora_core::lock` for the threat
    /// model this does and does not cover).
    app_locked: std::sync::atomic::AtomicBool,
    /// Unix time of the last request that counts as user activity, for
    /// `privacy.lock_after_idle_minutes`. Starts at daemon startup, not 0,
    /// so idle locking cannot fire immediately after a restart.
    last_activity: AtomicI64,
    /// Bumped on every real change to the live clipboard: `handle_event`
    /// (any backend, self-offers are already filtered out before it is
    /// called) and `recall`. `Arc` because CAP-07's delayed clear task is
    /// `tokio::spawn`ed detached from `self` (`Daemon` is held in an `Rc`,
    /// not `Send`) and needs its own handle to compare against later.
    capture_generation: Arc<AtomicU64>,
    /// Set just before CAP-07's delayed clear calls `backend.clear`, and
    /// consumed by the very next `OwnerGone` event `handle_event` sees.
    /// Without this, a deliberate clear looks exactly like a source
    /// application exiting (the selection is empty either way), so
    /// `persist_after_owner_gone` would immediately re-offer the entry the
    /// clear just removed.
    clearing_deliberately: Arc<std::sync::atomic::AtomicBool>,
}

/// How long a deleted entry can be brought back with `restore` before its
/// tombstone and blobs are purged for good.
pub const UNDO_GRACE_SECS: i64 = 30;

/// Blob MIME under which an image entry's list thumbnail is kept. Never
/// offered on the clipboard, never counted as a payload.
pub const THUMBNAIL_MIME: &str = "application/x-panora-thumbnail";

/// Records in one `SyncChanges` reply when the caller does not say.
const SYNC_DEFAULT_LIMIT: usize = 100;
/// Most records one `SyncChanges` reply may hold.
const SYNC_MAX_LIMIT: usize = 1000;
/// Payload bytes one `SyncChanges` reply aims to stay under; a single
/// entry larger than this still goes out, alone.
const SYNC_REPLY_BYTES: usize = 32 * 1024 * 1024;
/// Largest entry (all formats together) the feed hands out. Bigger ones
/// stay on this device: the peer's frame limit (`panora-sync`) must hold a
/// sealed page, and one entry that never fits would stall the feed.
const SYNC_MAX_ENTRY_BYTES: usize = 64 * 1024 * 1024;
/// Highest Lamport value accepted from a peer. Far beyond any real history
/// (one change per microsecond for 285 years), and low enough that the
/// local clock can never overflow by counting on from a hostile value.
const LAMPORT_CEILING: i64 = 1 << 53;

/// What became of one record handed to `Daemon::sync_apply`.
enum Applied {
    Yes,
    Ignored,
    Rejected,
}
/// Longest side of a thumbnail, in pixels; the popup draws them at 320×132.
const THUMBNAIL_MAX_SIDE: u32 = 320;

/// Decode an image payload off the async thread and shrink it to a PNG of
/// at most `THUMBNAIL_MAX_SIDE` on its longest side. Decoding is bounded
/// (dimensions and allocations) because the bytes come from any application;
/// anything the decoder cannot handle (SVG, damaged data) yields `None` and
/// the popup falls back to the full payload.
async fn render_thumbnail(bytes: Vec<u8>) -> Option<Vec<u8>> {
    tokio::task::spawn_blocking(move || thumbnail_png(&bytes))
        .await
        .ok()
        .flatten()
}

fn thumbnail_png(bytes: &[u8]) -> Option<Vec<u8>> {
    use std::io::Cursor;
    let mut reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16_384);
    limits.max_image_height = Some(16_384);
    limits.max_alloc = Some(256 * 1024 * 1024);
    reader.limits(limits);
    let decoded = reader.decode().ok()?;
    let small = if decoded.width() > THUMBNAIL_MAX_SIDE || decoded.height() > THUMBNAIL_MAX_SIDE {
        decoded.thumbnail(THUMBNAIL_MAX_SIDE, THUMBNAIL_MAX_SIDE)
    } else {
        decoded
    };
    let mut out = Cursor::new(Vec::new());
    small.write_to(&mut out, image::ImageFormat::Png).ok()?;
    Some(out.into_inner())
}

impl Daemon {
    /// Assemble a daemon from its parts.
    pub fn new(
        backend: Arc<dyn ClipboardBackend>,
        db: Database,
        blobs: BlobStore,
        config: Config,
        sync: Arc<dyn SyncProvider>,
        device_id: String,
    ) -> Self {
        let privacy = privacy_engine_for(&config);
        privacy.set_private_mode(config.privacy.start_private);
        Self {
            backend,
            privacy: RwLock::new(privacy),
            db,
            blobs,
            sync,
            config: RwLock::new(config),
            device_id,
            revision: AtomicU64::new(1),
            revision_watch: tokio::sync::watch::channel(1).0,
            last_stored: Mutex::new(HashMap::new()),
            last_recall: Mutex::new(None),
            config_epoch: tokio::sync::watch::channel(0).0,
            locked: std::sync::atomic::AtomicBool::new(false),
            extension_present: std::sync::atomic::AtomicBool::new(true),
            app_locked: std::sync::atomic::AtomicBool::new(false),
            last_activity: AtomicI64::new(unix_now()),
            capture_generation: Arc::new(AtomicU64::new(0)),
            clearing_deliberately: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    /// Whether the session is locked and recording paused because of it.
    pub fn locked(&self) -> bool {
        self.locked.load(Ordering::Relaxed)
    }

    /// Pause (or resume) recording while the screensaver is active.
    pub fn set_locked(&self, locked: bool) {
        if self.locked.swap(locked, Ordering::Relaxed) != locked {
            info!(locked, "session lock state changed");
            self.bump();
        }
    }

    /// Whether the GNOME Shell extension currently owns its bus name.
    pub fn extension_present(&self) -> bool {
        self.extension_present.load(Ordering::Relaxed)
    }

    /// Record the extension appearing on or leaving the bus; the revision
    /// moves so open popups pick the change up on their next poll.
    pub fn set_extension_present(&self, present: bool) {
        if self.extension_present.swap(present, Ordering::Relaxed) != present {
            info!(present, "GNOME Shell extension presence changed");
            self.bump();
        }
    }

    /// Findings the clients should put in front of the user.
    fn health(&self) -> Vec<HealthItem> {
        let mut items = Vec::new();
        if self.backend.capabilities().needs_bridge && !self.extension_present() {
            items.push(HealthItem {
                code: health::EXTENSION_MISSING.into(),
                message: format!(
                    "The GNOME Shell extension is not running, so nothing is recorded on \
                     this session. Enable it with: gnome-extensions enable {}",
                    health::EXTENSION_UUID
                ),
            });
        }
        if matches!(self.db.rotation_state(), Ok(Some(_))) {
            items.push(HealthItem {
                code: health::ROTATION_INCOMPLETE.into(),
                message: "A previous key rotation was interrupted before finishing; history is \
                          still fully readable. Finish it with: panora-cli rotate-key"
                    .into(),
            });
        }
        items
    }

    /// Whether recording is paused.
    pub fn private_mode(&self) -> bool {
        self.privacy
            .read()
            .map(|p| p.private_mode())
            .unwrap_or(false)
    }

    /// Pause or resume recording.
    pub fn set_private_mode(&self, on: bool) {
        if let Ok(privacy) = self.privacy.read() {
            privacy.set_private_mode(on);
        }
        self.bump();
        info!(private_mode = on, "private mode changed");
    }

    /// Access the database (for IPC queries).
    pub fn db(&self) -> &Database {
        &self.db
    }

    /// Access the blob store.
    pub fn blobs(&self) -> &BlobStore {
        &self.blobs
    }

    /// Access the backend.
    pub fn backend(&self) -> &Arc<dyn ClipboardBackend> {
        &self.backend
    }

    /// Snapshot of the current configuration.
    pub fn config(&self) -> Config {
        self.config.read().map(|c| c.clone()).unwrap_or_default()
    }

    /// Monotonic mutation counter for cheap client refresh checks.
    pub fn revision(&self) -> u64 {
        self.revision.load(Ordering::Relaxed)
    }

    fn bump(&self) {
        let revision = self.revision.fetch_add(1, Ordering::Relaxed) + 1;
        // No receivers (no `Subscribe` connection open) is not an error.
        let _ = self.revision_watch.send(revision);
    }

    /// A receiver that resolves every time `revision` changes; used by
    /// `Subscribe` connections (STO-08) instead of polling `Status`.
    pub fn watch_revision(&self) -> tokio::sync::watch::Receiver<u64> {
        self.revision_watch.subscribe()
    }

    /// Replace the configuration (from `config.toml`) without a restart.
    /// Private mode is user state, not configuration, so it is preserved.
    pub fn apply_config(&self, config: Config) -> Result<()> {
        config.validate()?;
        let private_now = self.private_mode();
        let privacy = privacy_engine_for(&config);
        privacy.set_private_mode(private_now);
        if let Ok(mut slot) = self.privacy.write() {
            *slot = privacy;
        }
        if let Ok(mut slot) = self.config.write() {
            *slot = config;
        }
        info!("configuration reloaded");
        self.config_epoch.send_modify(|epoch| *epoch += 1);
        self.collect_garbage()?;
        self.bump();
        Ok(())
    }

    /// Re-read `config.toml` and apply it.
    pub fn reload_config(&self) -> Result<()> {
        self.apply_config(Config::load()?)
    }

    /// SEC-01: generate a new master key, reseal every stored preview and
    /// blob under it, then retire the old one.
    ///
    /// Ordered so a crash at any point leaves the history fully readable
    /// under *some* key this process (or a resumed one) still has: the new
    /// key is written to the keyring's pending slot before anything is
    /// resealed, `Database::rekey`/`BlobStore::rekey` are individually
    /// idempotent (each re-checks what is already under the new key rather
    /// than assuming nothing is), and the live keyring item is only
    /// replaced — and the pending one only deleted — after both succeed.
    /// Calling this again after an interruption picks the same pending key
    /// back up instead of generating a new one, so it finishes the same
    /// rotation rather than starting a second one on top of it.
    pub async fn rotate_key(&self) -> Result<()> {
        let key = match crate::keyring::load_pending_key().await? {
            Some(key) => key,
            None => {
                let key = panora_core::storage::MasterKey::generate();
                crate::keyring::store_pending_key(&key).await?;
                key
            }
        };
        self.db.set_rotation_state(Some("rotating"))?;
        self.db.rekey(panora_core::storage::Cipher::new(&key))?;
        self.blobs.rekey(panora_core::storage::Cipher::new(&key))?;
        crate::keyring::finish_rotation(&key).await?;
        self.db.set_rotation_state(None)?;
        info!("master key rotation finished");
        self.bump();
        Ok(())
    }

    // --- SEC-02: second-layer password lock. See `panora_core::lock`'s
    // module docs for exactly what this does and does not protect against;
    // the short version is that it gates *use* of a running, unlocked
    // daemon, not the master key's rest-state in the keyring, which must
    // stay loadable unattended for `panod` to survive a reboot.

    /// Whether the second-layer lock is currently engaged. `List` degrades
    /// to a count and `Preview`/`Recall` are refused while this is true;
    /// gated in `server::handle_request`, not here, since it is about IPC
    /// access, not the daemon's own internal ability to read its storage.
    pub fn is_app_locked(&self) -> bool {
        self.app_locked.load(Ordering::Relaxed)
    }

    /// Whether a lock password has been set at all (independent of whether
    /// it is currently engaged).
    pub fn lock_password_set(&self) -> Result<bool> {
        Ok(self.db.lock_secret()?.is_some())
    }

    /// Record a request as user activity, for idle locking. Called from
    /// `handle_request` for everything except `Status`/`Hello` (health
    /// checks and negotiation should not by themselves keep the lock from
    /// engaging) and `Subscribe` (a long-lived connection that never
    /// repeats this call anyway).
    pub fn touch_activity(&self) {
        self.last_activity.store(unix_now(), Ordering::Relaxed);
    }

    /// Seconds since the last recorded activity.
    fn idle_seconds(&self) -> i64 {
        (unix_now() - self.last_activity.load(Ordering::Relaxed)).max(0)
    }

    /// Engage the idle lock if enough time has passed, a password is set,
    /// and it is not already engaged. Called periodically by `server`; a
    /// no-op most of the time, which is why it is infallible (an error
    /// here — a locked database, say — should not take a ticker task down).
    pub fn maybe_auto_lock(&self) {
        if self.is_app_locked() {
            return;
        }
        let idle_minutes = self.config().privacy.lock_after_idle_minutes;
        if idle_minutes == 0 {
            return;
        }
        if self.idle_seconds() < i64::from(idle_minutes) * 60 {
            return;
        }
        match self.lock_password_set() {
            Ok(true) => {
                self.app_locked.store(true, Ordering::Relaxed);
                info!(
                    idle_minutes,
                    "engaged the second-layer lock after idle timeout"
                );
                self.bump();
            }
            Ok(false) => {} // nothing to lock with
            Err(e) => warn!(error = %e, "could not check the lock password while idle"),
        }
    }

    /// Test-only: make `idle_seconds` report `seconds_ago` without a real
    /// wait, so `maybe_auto_lock` is testable on a fast clock.
    #[cfg(test)]
    fn backdate_activity_for_test(&self, seconds_ago: i64) {
        self.last_activity
            .store(unix_now() - seconds_ago, Ordering::Relaxed);
    }

    /// Engage the lock by hand (`panora-cli lock`). Refuses when no
    /// password is set — locking with nothing that can unlock it again
    /// would strand the user's own history.
    pub fn engage_lock(&self) -> Result<()> {
        if !self.lock_password_set()? {
            return Err(Error::Ipc(
                "no lock password is set; set one with: panora-cli lock set-password".into(),
            ));
        }
        self.app_locked.store(true, Ordering::Relaxed);
        self.bump();
        Ok(())
    }

    /// Check `password` against the stored verifier and disengage the lock
    /// on success.
    pub fn unlock(&self, password: &str) -> Result<()> {
        let secret = self
            .db
            .lock_secret()?
            .ok_or_else(|| Error::Ipc("no lock password is set".into()))?;
        if !secret.verify(password) {
            return Err(Error::Ipc("incorrect password".into()));
        }
        self.app_locked.store(false, Ordering::Relaxed);
        self.touch_activity();
        self.bump();
        Ok(())
    }

    /// Set, change or remove the lock password.
    ///
    /// Changing or removing an existing one requires `current_password` to
    /// verify first, regardless of whether the lock happens to be engaged
    /// right now — otherwise anyone with a moment of unlocked access could
    /// silently disable a lock they do not actually know the password for.
    /// Setting a *new* password also writes a password-gated backup copy of
    /// the live master key to the keyring (`crate::keyring::
    /// store_lock_backup`); refuses while a key rotation (SEC-01) is
    /// still in progress, since the "live" key the keyring would hand back
    /// right now might not be the one storage ends up under.
    pub async fn set_lock_password(
        &self,
        new_password: Option<&str>,
        current_password: Option<&str>,
    ) -> Result<()> {
        if let Some(existing) = self.db.lock_secret()? {
            let current = current_password
                .ok_or_else(|| Error::Ipc("the current password is required".into()))?;
            if !existing.verify(current) {
                return Err(Error::Ipc("incorrect password".into()));
            }
        }
        match new_password {
            Some(new_password) => {
                if self.db.rotation_state()?.is_some() {
                    return Err(Error::Ipc(
                        "a key rotation is in progress; finish it first with: panora-cli \
                         rotate-key"
                            .into(),
                    ));
                }
                let secret = panora_core::lock::LockSecret::new(new_password)?;
                let live_key = crate::keyring::load_or_create_master_key().await?;
                let wrapped = secret.wrap(new_password, &live_key)?;
                self.db.set_lock_secret(Some(&secret))?;
                crate::keyring::store_lock_backup(&wrapped).await?;
                info!("lock password set");
            }
            None => {
                self.db.set_lock_secret(None)?;
                crate::keyring::remove_lock_backup().await?;
                self.app_locked.store(false, Ordering::Relaxed);
                info!("lock password removed");
            }
        }
        self.bump();
        Ok(())
    }

    /// Panic wipe (SEC-03): hard-delete the whole history and retire the
    /// key that encrypted it, in that order, so nothing depends on the old
    /// key by the time it is destroyed. A fresh key is generated and
    /// adopted immediately afterward — by both storage (`rekey`, so a
    /// memory dump taken after this returns cannot recover the old key
    /// through the running process either) and the keyring — rather than
    /// leaving the daemon with no key at all, which would need someone
    /// there to supply one before capture could resume.
    ///
    /// Deliberately not gated by `is_app_locked`: the whole point of a
    /// panic action is that it still works under duress, not only after
    /// unlocking first.
    pub async fn wipe(&self) -> Result<()> {
        self.db.wipe()?;
        self.blobs.wipe()?;
        let fresh = panora_core::storage::MasterKey::generate();
        self.db.rekey(panora_core::storage::Cipher::new(&fresh))?;
        self.blobs
            .rekey(panora_core::storage::Cipher::new(&fresh))?;
        crate::keyring::wipe_and_replace(&fresh).await?;
        self.app_locked.store(false, Ordering::Relaxed);
        if let Ok(mut slot) = self.last_stored.lock() {
            slot.clear();
        }
        if let Ok(mut slot) = self.last_recall.lock() {
            *slot = None;
        }
        info!("panic wipe completed");
        self.bump();
        Ok(())
    }

    /// CLI-03: export the whole history (pinned and unpinned alike,
    /// tombstoned entries excluded the same way `query` excludes them by
    /// default) as an encrypted archive `import` can restore from.
    pub fn export(&self, passphrase: &str) -> Result<Vec<u8>> {
        let entries = self.db.query(&QueryFilter::recent(usize::MAX))?;
        let mut records = Vec::with_capacity(entries.len());
        for entry in entries {
            let payloads = self.load_payloads(entry.id)?;
            let record = panora_core::backup::BackupEntry {
                content_hash: entry.content_hash,
                preview: entry.preview,
                kind: entry.kind.as_str().to_string(),
                primary_mime: entry.primary_mime,
                size_bytes: entry.size_bytes,
                source_app: entry.source_app,
                selection: entry.selection.as_str().to_string(),
                created_at: entry.created_at,
                last_seen_at: entry.last_seen_at,
                pinned: entry.pinned,
                sensitive: entry.sensitive,
                payloads: payloads
                    .iter()
                    .map(|p| (p.mime.clone(), panora_core::storage::content_hash(&p.data)))
                    .collect(),
            };
            records.push((record, payloads));
        }
        panora_core::backup::build_archive(&records, passphrase)
    }

    /// CLI-03: restore entries from an archive `export` produced. Bypasses
    /// the privacy gate — the archive is the user's own previously-
    /// exported data, not a live capture to be screened — and deduplicates
    /// by content hash exactly like one: importing an entry that is
    /// already present (the same archive twice, or overlapping backups)
    /// bumps it instead of duplicating it. Returns the number of entries
    /// processed (imported or deduplicated into an existing row alike).
    pub async fn import(&self, passphrase: &str, archive: &[u8]) -> Result<usize> {
        let records = panora_core::backup::open_archive(passphrase, archive)?;
        let count = records.len();
        for (record, payloads) in records {
            self.import_entry(&record, payloads).await?;
        }
        if count > 0 {
            self.bump();
        }
        Ok(count)
    }

    async fn import_entry(
        &self,
        record: &panora_core::backup::BackupEntry,
        payloads: Vec<MimePayload>,
    ) -> Result<Entry> {
        let kind = ContentKind::parse(&record.kind);
        let selection = if record.selection == Selection::Primary.as_str() {
            Selection::Primary
        } else {
            Selection::Clipboard
        };
        let mut hasher = blake3::Hasher::new();
        for p in &payloads {
            hasher.update(p.mime.as_bytes());
            hasher.update(&p.data);
        }
        let content_hash = hasher.finalize().to_hex().to_string();
        let lamport = self.db.tick_lamport()?;
        // `upsert_entry` only takes one timestamp for a fresh row (used for
        // both created_at and last_seen_at); last_seen_at is what drives
        // sort order and freshness, so a restored entry keeps that one
        // exactly and created_at collapses to it rather than the true
        // original creation time.
        let id = self.db.upsert_entry(
            &content_hash,
            &record.preview,
            kind,
            &record.primary_mime,
            record.size_bytes,
            record.source_app.as_deref(),
            selection,
            record.last_seen_at,
            &self.device_id,
            lamport,
            // A restore is not a live re-copy the user would leave in
            // place; `duplicate_policy` governs capture, not import.
            true,
        )?;
        self.db.stamp(id, lamport, &self.device_id)?;
        if record.sensitive {
            self.db.mark_sensitive(id)?;
        } else if self.config().history.index_full_text {
            let data = ClipboardData {
                selection,
                offered_mimes: payloads.iter().map(|p| p.mime.clone()).collect(),
                payloads: payloads.clone(),
                source_app: record.source_app.clone(),
            };
            if let Some(text) = data.text() {
                if let Some(content) = index_text(&text, kind) {
                    self.db.index_content(id, &content)?;
                }
            }
        }
        for payload in &payloads {
            let blob_ref = self.blobs.put(&payload.data)?;
            self.db.attach_blob(id, &payload.mime, &blob_ref)?;
        }
        if record.pinned {
            self.db.set_pinned(id, true)?;
        }
        self.db.get(id)
    }

    /// Status report for IPC clients.
    pub fn status(&self) -> Result<StatusData> {
        let caps = self.backend.capabilities();
        Ok(StatusData {
            backend: self.backend.name().into(),
            entries: self.db.count()?,
            private_mode: self.private_mode(),
            sync_active: self.sync.active(),
            revision: self.revision(),
            version: env!("CARGO_PKG_VERSION").into(),
            protocol: PROTOCOL_VERSION,
            capabilities: CapabilityData {
                primary: caps.primary,
                images: caps.images,
                persist: caps.persist,
                synthetic_paste: caps.synthetic_paste,
                source_app: caps.source_app,
                needs_bridge: caps.needs_bridge,
            },
            locked: self.locked(),
            app_locked: self.is_app_locked(),
            lock_password_set: self.lock_password_set()?,
            health: self.health(),
        })
    }

    /// Aggregate history statistics for `panora-cli stats` (CLI-06).
    pub fn stats(&self) -> Result<panora_core::ipc::StatsData> {
        self.db.stats()
    }

    /// Run the capture loop until the shutdown channel closes.
    pub async fn run(&self, mut shutdown: mpsc::Receiver<()>) -> Result<()> {
        let mut rx = self.backend.watch(Selection::Clipboard).await?;
        let mut config_rx = self.config_epoch.subscribe();
        let mut primary_rx = self.primary_watch().await?;

        info!(backend = self.backend.name(), "panod capture loop started");

        loop {
            tokio::select! {
                _ = shutdown.recv() => {
                    info!("shutdown requested, stopping capture loop");
                    return Ok(());
                }
                ev = rx.recv() => {
                    match ev {
                        Some(e) => self.handle_event(e).await,
                        None => {
                            return Err(Error::Backend(
                                "clipboard watch ended (display connection lost)".into(),
                            ));
                        }
                    }
                }
                changed = config_rx.changed() => {
                    if changed.is_err() {
                        // The sender lives in `self`, so this cannot happen
                        // while the loop runs; stay defensive anyway.
                        continue;
                    }
                    // `record_primary` is the one setting the loop owns: the
                    // watch has to be opened or dropped here, not merely
                    // consulted per event, or the toggle needs a restart.
                    let wanted = self.config().history.record_primary;
                    match (wanted, primary_rx.is_some()) {
                        (true, false) => match self.primary_watch().await {
                            Ok(watch) => primary_rx = watch,
                            Err(e) => warn!(error = %e, "cannot start PRIMARY watch"),
                        },
                        (false, true) => {
                            primary_rx = None;
                            info!("PRIMARY recording disabled");
                        }
                        _ => {}
                    }
                }
                ev = async {
                    match primary_rx.as_mut() {
                        Some(r) => r.recv().await,
                        None => std::future::pending().await,
                    }
                } => {
                    match ev {
                        Some(e) => self.handle_event(e).await,
                        None => primary_rx = None,
                    }
                }
            }
        }
    }

    /// Open the PRIMARY watch when recording it is both wanted and possible.
    async fn primary_watch(&self) -> Result<Option<mpsc::Receiver<ClipboardEvent>>> {
        if self.config().history.record_primary && self.backend.capabilities().primary {
            let rx = self.backend.watch(Selection::Primary).await?;
            info!("PRIMARY recording enabled");
            Ok(Some(rx))
        } else {
            Ok(None)
        }
    }

    /// Process one clipboard change event: privacy gate first, then
    /// payload read, then encrypted storage, then sync notification.
    pub async fn handle_event(&self, event: ClipboardEvent) {
        // Any event reaching here is a real change of the live clipboard:
        // backends already filter out panod's own offers before producing
        // one (X11's watcher never sees its own ownership change; Wayland's
        // RECALL_MARKER_MIME and the GNOME bridge's `is_echo_of_recall`
        // catch the rest). CAP-07's delayed clear compares against this so
        // it never wipes content that arrived after the recall it belongs
        // to, even when the new content happens to offer the exact same
        // MIME types (`same_targets` alone cannot tell those apart).
        self.capture_generation.fetch_add(1, Ordering::Relaxed);
        if event.kind == EventKind::OwnerGone {
            if self.clearing_deliberately.swap(false, Ordering::Relaxed) {
                debug!("selection emptied by our own clear, not re-offering");
                self.forget_last_stored(event.selection);
            } else if self.persistence_enabled() {
                self.persist_after_owner_gone(event.selection).await;
            } else {
                self.forget_last_stored(event.selection);
            }
            return;
        }
        // Whatever happens below, a change that is not stored must not be
        // "restored" later by the persistence path.
        self.forget_last_stored(event.selection);
        if self.locked() {
            debug!("session locked; change not recorded");
            return;
        }
        if event.selection == Selection::Primary && !self.config().history.record_primary {
            return;
        }

        // Gate 1+2: evaluate policy using ONLY the offered MIME list and
        // the source app name. Payloads have not been read yet (ADR 0003).
        let probe = ClipboardData {
            selection: event.selection,
            payloads: Vec::new(),
            offered_mimes: event.offered_mimes.clone(),
            source_app: event.source_app.clone(),
        };
        let verdict = self
            .privacy
            .read()
            .map(|p| p.evaluate(&probe))
            .unwrap_or(panora_core::privacy::Verdict::RejectPrivateMode);
        if !verdict.is_allowed() {
            debug!(?verdict, "content rejected by privacy policy");
            return;
        }
        // The focused window's title, when the backend knows it, is judged
        // here too and then forgotten.
        let title_verdict = self
            .privacy
            .read()
            .map(|p| p.evaluate_title(event.source_title.as_deref()))
            .unwrap_or(panora_core::privacy::Verdict::RejectPrivateMode);
        if !title_verdict.is_allowed() {
            debug!(?title_verdict, "content rejected by the window title list");
            return;
        }

        let limit = self.config().history.max_mime_bytes;
        let mut payloads: Vec<MimePayload> = Vec::new();
        for mime in wanted_order(&event.offered_mimes) {
            match self.backend.read(event.selection, &mime).await {
                Ok(bytes) => {
                    if bytes.is_empty() {
                        continue;
                    }
                    if bytes.len() > limit {
                        warn!(
                            mime,
                            size = bytes.len(),
                            limit,
                            "payload exceeds size limit, skipping"
                        );
                        continue;
                    }
                    payloads.push(MimePayload { mime, data: bytes });
                }
                Err(e) => {
                    debug!(mime, error = %e, "failed to read payload");
                }
            }
        }
        if payloads.is_empty() {
            return;
        }

        // The owner may have changed while payloads were being read, in
        // which case the bytes above belong to a selection the privacy
        // engine never evaluated. Re-read TARGETS and drop the capture on
        // any difference; the new owner produces its own event.
        if let Ok(current) = self.backend.read_targets(event.selection).await {
            if !same_targets(&current, &event.offered_mimes) {
                debug!("selection owner changed during read; discarding capture");
                return;
            }
        }

        let data = ClipboardData {
            selection: event.selection,
            payloads,
            offered_mimes: event.offered_mimes.clone(),
            source_app: event.source_app.clone(),
        };
        // Gate 3: the user's content filters, now that the text is known.
        if !self.content_allowed(&data) {
            return;
        }
        match self.store(data).await {
            Ok(entry) => self.remember_last_stored(event.selection, entry.id),
            Err(e) => warn!(error = %e, "failed to store clipboard event"),
        }
    }

    fn remember_last_stored(&self, selection: Selection, id: i64) {
        if let Ok(mut slot) = self.last_stored.lock() {
            slot.insert(selection, id);
        }
    }

    fn forget_last_stored(&self, selection: Selection) {
        if let Ok(mut slot) = self.last_stored.lock() {
            slot.remove(&selection);
        }
    }

    /// The second privacy gate: kind, length, whitespace and ignore-pattern
    /// filters, judged once the payloads are in hand.
    fn content_allowed(&self, data: &ClipboardData) -> bool {
        let verdict = self
            .privacy
            .read()
            .map(|p| p.evaluate_content(data))
            .unwrap_or(panora_core::privacy::Verdict::RejectPrivateMode);
        if !verdict.is_allowed() {
            debug!(?verdict, "content rejected by the content filters");
        }
        verdict.is_allowed()
    }

    /// Whether an empty clipboard after the owner left should be refilled.
    /// X11 always (it has no persistence at all); on Wayland the backend's
    /// compositor detection decides unless `persist_on_wayland` overrides it.
    fn persistence_enabled(&self) -> bool {
        if self.backend.name() != "wayland" {
            return true;
        }
        match self.config().history.persist_on_wayland.as_str() {
            "always" => true,
            "never" => false,
            _ => self.backend.capabilities().persist,
        }
    }

    /// The selection owner exited and took the content with it (X11 has no
    /// persistence of its own). Re-offer that content -- and only that
    /// content: the entry stored from the very last change. A change that
    /// was rejected, cleared or never stored leaves nothing to restore, so a
    /// password manager clearing the clipboard is never undone.
    async fn persist_after_owner_gone(&self, selection: Selection) {
        if selection != Selection::Clipboard {
            return;
        }
        let id = self
            .last_stored
            .lock()
            .ok()
            .and_then(|mut slot| slot.remove(&selection));
        let Some(id) = id else {
            return;
        };
        let Ok(entry) = self.db.get(id) else {
            return;
        };
        if entry.deleted {
            return;
        }
        match self.offer_entry(&entry).await {
            Ok(()) => debug!(id = entry.id, "re-offered last entry after owner exit"),
            Err(e) => debug!(error = %e, "could not re-offer last entry"),
        }
    }

    /// Accept data forwarded by the GNOME Shell extension. The extension
    /// has already read a payload because Mutter requires it, so this
    /// method immediately re-applies the same TARGETS-first privacy
    /// policy before any storage operation.
    pub async fn handle_gnome_data(&self, data: ClipboardData, window_title: Option<String>) {
        if !self.backend.capabilities().needs_bridge {
            // A native data-control backend already captured this change
            // with every format; storing the single bridge payload too would
            // create a second, poorer entry.
            debug!("GNOME bridge payload ignored: native backend is active");
            return;
        }
        if self.locked() {
            debug!("session locked; GNOME bridge payload not recorded");
            return;
        }
        let probe = ClipboardData {
            selection: data.selection,
            payloads: Vec::new(),
            offered_mimes: data.offered_mimes.clone(),
            source_app: data.source_app.clone(),
        };
        let verdict = self
            .privacy
            .read()
            .map(|p| p.evaluate(&probe))
            .unwrap_or(panora_core::privacy::Verdict::RejectPrivateMode);
        if !verdict.is_allowed() {
            debug!(?verdict, "GNOME bridge content rejected by privacy policy");
            return;
        }
        let title_verdict = self
            .privacy
            .read()
            .map(|p| p.evaluate_title(window_title.as_deref()))
            .unwrap_or(panora_core::privacy::Verdict::RejectPrivateMode);
        if !title_verdict.is_allowed() {
            debug!(?title_verdict, "content rejected by the window title list");
            return;
        }

        // The capture path enforces max_mime_bytes when it reads a payload,
        // but bridge payloads arrive pre-read over D-Bus. Without the same
        // check any session-bus caller could push oversized blobs straight
        // into the encrypted store and grow it without bound.
        let limit = self.config().history.max_mime_bytes;
        let mut data = data;
        data.payloads.retain(|payload| {
            if payload.data.len() > limit {
                warn!(
                    mime = %payload.mime,
                    size = payload.data.len(),
                    limit,
                    "GNOME bridge payload exceeds size limit, skipping"
                );
                return false;
            }
            !payload.data.is_empty()
        });
        if data.payloads.is_empty() {
            return;
        }
        if let Some(id) = self.is_echo_of_recall(&data) {
            debug!(id, "GNOME bridge echoed our own recall; not stored again");
            let _ = self.db.touch(id, unix_now());
            return;
        }
        // Past this point the bridge is reporting a genuine external
        // change (see the comment on `capture_generation` in `handle_event`
        // — the bridge is the one backend that needs its own echo filter
        // before it can say the same).
        self.capture_generation.fetch_add(1, Ordering::Relaxed);

        if !self.content_allowed(&data) {
            return;
        }
        match self.store(data).await {
            Ok(entry) => self.remember_last_stored(Selection::Clipboard, entry.id),
            Err(e) => warn!(error = %e, "failed to store GNOME bridge clipboard event"),
        }
    }

    /// If `data` carries exactly the bytes of the entry recalled within the
    /// last few seconds, return that entry's id. Blob references are BLAKE3
    /// hashes of the plaintext, so no payload needs to be decrypted.
    fn is_echo_of_recall(&self, data: &ClipboardData) -> Option<i64> {
        let (id, at) = (*self.last_recall.lock().ok()?)?;
        if at.elapsed() > RECALL_ECHO_WINDOW {
            return None;
        }
        let refs = self.db.blobs_of(id).ok()?;
        let echoed = data.payloads.iter().all(|p| {
            let hash = panora_core::storage::content_hash(&p.data);
            refs.iter().any(|(_, blob_ref)| *blob_ref == hash)
        });
        echoed.then_some(id)
    }

    /// Persist captured data: dedup by content hash, encrypt payloads
    /// into the blob store, index metadata + preview in SQLite/FTS5.
    pub async fn store(&self, data: ClipboardData) -> Result<Entry> {
        let kind = data.classify();
        let text = data.text().unwrap_or_default();
        let preview = make_preview(&text, &data);
        // Secrets: `mask` records them behind a masked preview and keeps
        // them out of the full-text index, `store` records them as they are
        // but flagged; `drop` was applied by the content filters already.
        let sensitive = matches!(kind, ContentKind::Text | ContentKind::RichText)
            .then(|| panora_core::sensitive::detect(&text))
            .flatten();
        let masked = sensitive.is_some() && self.config().privacy.sensitive_policy == "mask";
        let preview = match sensitive {
            Some(found) if masked => format!("{SENSITIVE_MASK} {}", found.label()),
            _ => preview,
        };
        let primary_mime = data
            .payloads
            .first()
            .map(|p| p.mime.clone())
            .unwrap_or_else(|| "application/octet-stream".to_string());

        // Hash over all payloads concatenated for stable identity.
        let mut hasher = blake3::Hasher::new();
        for p in &data.payloads {
            hasher.update(p.mime.as_bytes());
            hasher.update(&p.data);
        }
        let content_hash = hasher.finalize().to_hex().to_string();

        let now = unix_now();
        let lamport = self.db.tick_lamport()?;

        let id = self.db.upsert_entry(
            &content_hash,
            &preview,
            kind,
            &primary_mime,
            data.total_size() as i64,
            data.source_app.as_deref(),
            data.selection,
            now,
            &self.device_id,
            lamport,
            self.config().history.duplicate_policy != "ignore",
        )?;
        // A re-copy of an existing row keeps its old stamp in
        // `upsert_entry`; it is a new state all the same (SYNC-03).
        self.db.stamp(id, lamport, &self.device_id)?;

        if sensitive.is_some() {
            self.db.mark_sensitive(id)?;
        }

        // Search beyond the 500-character preview: the text itself goes
        // into the FTS table (same 0600 file as the previews) when enabled.
        if self.config().history.index_full_text && !masked {
            if let Some(content) = index_text(&text, kind) {
                self.db.index_content(id, &content)?;
            }
        }

        // Store payloads as encrypted blobs and attach references.
        for p in &data.payloads {
            let blob_ref = self.blobs.put(&p.data)?;
            self.db.attach_blob(id, &p.mime, &blob_ref)?;
        }
        // Images get a small thumbnail now, so the popup never has to pull
        // and decode megabytes per row just to draw a list.
        if kind == ContentKind::Image {
            if let Some(image) = data.payloads.iter().find(|p| p.mime.starts_with("image/")) {
                match render_thumbnail(image.data.clone()).await {
                    Some(png) => {
                        let blob_ref = self.blobs.put(&png)?;
                        self.db.attach_blob(id, THUMBNAIL_MIME, &blob_ref)?;
                    }
                    None => debug!(id, mime = %image.mime, "no thumbnail for this image"),
                }
            }
        }

        self.collect_garbage()?;
        self.bump();

        let entry = self.db.get(id)?;
        self.sync
            .on_event(SyncEvent::EntryUpserted(entry.clone()))
            .await;
        Ok(entry)
    }

    /// Apply retention policies and remove unreferenced blobs from disk.
    pub fn collect_garbage(&self) -> Result<usize> {
        let config = self.config();
        let now = unix_now();
        let mut evicted = self.db.enforce_limit(config.history.max_entries)?;
        if config.history.max_age_days > 0 {
            let cutoff = now - (i64::from(config.history.max_age_days) * 86_400);
            evicted.extend(self.db.enforce_age(cutoff)?);
        }
        if config.privacy.sensitive_ttl_minutes > 0 {
            let cutoff = now - i64::from(config.privacy.sensitive_ttl_minutes) * 60;
            evicted.extend(self.db.expire_sensitive(cutoff)?);
        }
        if config.history.max_total_bytes > 0 {
            evicted.extend(
                self.db
                    .enforce_total_bytes(config.history.max_total_bytes)?,
            );
        }
        // Evictions (deleted_at = 0) go right away. A user's deletion lives
        // long enough for an undo; with sync on (SYNC-04) its row then stays,
        // without payloads, for `sync.tombstone_days`, so a device that was
        // offline learns about the deletion instead of sending the entry
        // back.
        if config.sync.enabled {
            for blob_ref in self.db.strip_tombstones(now - UNDO_GRACE_SECS)? {
                if let Err(e) = self.blobs.remove(&blob_ref) {
                    warn!(blob = %blob_ref, error = %e, "blob cleanup failed");
                }
            }
            let keep = i64::from(config.sync.tombstone_days) * 86_400;
            self.purge_tombstoned_before(now - keep)?;
        } else {
            self.purge_tombstoned_before(now - UNDO_GRACE_SECS)?;
        }
        if !evicted.is_empty() {
            debug!(count = evicted.len(), "evicted entries by retention policy");
        }
        Ok(evicted.len())
    }

    /// The hourly upkeep: retention, database maintenance and a scan for
    /// blobs no row references (left by a crash between writing a blob and
    /// recording it). Returns how many orphans were removed.
    pub fn maintain(&self) -> Result<usize> {
        self.collect_garbage()?;
        self.db.maintain()?;
        let referenced = self.db.referenced_blobs()?;
        let mut removed = 0usize;
        for hash in self.blobs.list()? {
            if !referenced.contains(&hash) {
                match self.blobs.remove(&hash) {
                    Ok(()) => removed += 1,
                    Err(e) => warn!(blob = %hash, error = %e, "orphan blob cleanup failed"),
                }
            }
        }
        if removed > 0 {
            info!(removed, "removed orphaned blobs");
        }
        Ok(removed)
    }

    /// Drop the rows of entries tombstoned before `before`, then the blobs
    /// nothing references any more.
    pub fn purge_tombstoned_before(&self, before: i64) -> Result<()> {
        for blob_ref in self.db.purge_tombstones(before)? {
            if let Err(e) = self.blobs.remove(&blob_ref) {
                warn!(blob = %blob_ref, error = %e, "blob cleanup failed");
            }
        }
        Ok(())
    }

    /// Load an entry's payloads back from the blob store.
    pub fn load_payloads(&self, entry_id: i64) -> Result<Vec<MimePayload>> {
        let entry = self.db.get(entry_id)?;
        if entry.deleted {
            return Err(Error::NotFound(entry_id));
        }
        let mut out = Vec::new();
        for (mime, blob_ref) in self.db.blobs_of(entry_id)? {
            // The thumbnail is Panora's own; it is never a clipboard format.
            if mime == THUMBNAIL_MIME {
                continue;
            }
            let data = self.blobs.get(&blob_ref)?;
            out.push(MimePayload { mime, data });
        }
        // Keep the daemon's preference order so clients and backends see a
        // stable "primary" format first.
        out.sort_by_key(|p| mime_rank(&p.mime));
        Ok(out)
    }

    /// The stored thumbnail of an image entry, if one exists.
    pub fn load_thumbnail(&self, entry_id: i64) -> Result<Option<MimePayload>> {
        for (mime, blob_ref) in self.db.blobs_of(entry_id)? {
            if mime == THUMBNAIL_MIME {
                return Ok(Some(MimePayload {
                    mime,
                    data: self.blobs.get(&blob_ref)?,
                }));
            }
        }
        Ok(None)
    }

    /// What the popup draws for a row: the thumbnail of an image entry
    /// (made on the spot for entries captured before thumbnails existed, or
    /// whose format could not be decoded then), or the full payloads.
    pub async fn thumbnail_or_full(&self, entry_id: i64) -> Result<Vec<MimePayload>> {
        let entry = self.db.get(entry_id)?;
        if entry.deleted {
            return Err(Error::NotFound(entry_id));
        }
        if entry.kind != ContentKind::Image {
            return self.load_payloads(entry_id);
        }
        if let Some(thumbnail) = self.load_thumbnail(entry_id)? {
            return Ok(vec![thumbnail]);
        }
        let full = self.load_payloads(entry_id)?;
        let Some(image) = full.iter().find(|p| p.mime.starts_with("image/")) else {
            return Ok(full);
        };
        match render_thumbnail(image.data.clone()).await {
            Some(png) => {
                let blob_ref = self.blobs.put(&png)?;
                self.db.attach_blob(entry_id, THUMBNAIL_MIME, &blob_ref)?;
                Ok(vec![MimePayload::new(THUMBNAIL_MIME, png)])
            }
            None => Ok(full),
        }
    }

    /// Offer an entry on the CLIPBOARD selection. Entries captured from
    /// PRIMARY are recalled to the clipboard too: that is what the user
    /// pastes with Ctrl+V.
    async fn offer_entry(&self, entry: &Entry) -> Result<()> {
        self.offer_entry_as(entry, None, Selection::Clipboard).await
    }

    /// Like `offer_entry`, restricted to one format when `only` is given
    /// (a text request also accepts the other plain-text aliases), and
    /// targeting `to` instead of always CLIPBOARD (CAP-10: recall to
    /// PRIMARY for a middle-click paste).
    async fn offer_entry_as(&self, entry: &Entry, only: Option<&str>, to: Selection) -> Result<()> {
        let mut payloads = self.load_payloads(entry.id)?;
        if let Some(only) = only {
            let exact: Vec<MimePayload> = payloads
                .iter()
                .filter(|p| p.mime.eq_ignore_ascii_case(only))
                .cloned()
                .collect();
            payloads = if !exact.is_empty() {
                exact
            } else if panora_core::model::is_text_mime(only) {
                panora_core::model::TEXT_MIMES
                    .iter()
                    .find_map(|m| payloads.iter().find(|p| p.mime == *m))
                    .or_else(|| payloads.iter().find(|p| p.is_text()))
                    .cloned()
                    .into_iter()
                    .collect()
            } else {
                Vec::new()
            };
        }
        if payloads.is_empty() {
            return Err(Error::Backend(format!(
                "entry {} has no payload for the requested format",
                entry.id
            )));
        }
        let data = ClipboardData {
            selection: to,
            offered_mimes: payloads.iter().map(|p| p.mime.clone()).collect(),
            source_app: Some("panora".into()),
            payloads,
        };
        self.backend.offer(to, data).await
    }

    /// Put a history entry back on `to`, then optionally paste it. `paste`
    /// only does anything for `Selection::Clipboard`: there is no keyboard
    /// shortcut for a PRIMARY paste, only middle-click, so it is silently
    /// ignored for `Selection::Primary`.
    pub async fn recall(
        &self,
        entry_id: i64,
        paste: bool,
        mime: Option<&str>,
        to: Selection,
    ) -> Result<RecallOutcome> {
        let entry = self.db.get(entry_id)?;
        if entry.deleted {
            return Err(Error::NotFound(entry_id));
        }
        self.offer_entry_as(&entry, mime, to).await?;
        // Our own change, so `handle_event`/`handle_gnome_data` will not
        // see it (every backend filters its own echo before producing
        // one) — bump here instead, so CAP-07's clear timer, captured
        // right after this, has an up-to-date baseline to compare against.
        self.capture_generation.fetch_add(1, Ordering::Relaxed);
        if let Ok(mut slot) = self.last_recall.lock() {
            *slot = Some((entry_id, std::time::Instant::now()));
        }
        self.db.touch(entry_id, unix_now())?;
        self.bump();
        self.schedule_clipboard_clear(to).await;
        if !paste || to != Selection::Clipboard {
            return Ok(RecallOutcome { pasted: false });
        }
        tokio::time::sleep(PASTE_DELAY).await;
        match self.backend.synthetic_paste().await {
            Ok(()) => Ok(RecallOutcome { pasted: true }),
            Err(e) => {
                warn!(error = %e, "synthetic paste failed");
                Ok(RecallOutcome { pasted: false })
            }
        }
    }

    /// CAP-07: if `privacy.clear_clipboard_after_seconds` is set, clear the
    /// live clipboard that many seconds after a recall — the way a password
    /// manager times out what it put on the clipboard. Fires only if
    /// `capture_generation` is still exactly what it was right after the
    /// recall's own offer; comparing MIME lists (`read_targets`) alone
    /// cannot tell two different plain-text copies apart, since both
    /// advertise the same targets.
    async fn schedule_clipboard_clear(&self, selection: Selection) {
        let seconds = self.config().privacy.clear_clipboard_after_seconds;
        if seconds == 0 {
            return;
        }
        let generation = self.capture_generation.clone();
        let baseline = generation.load(Ordering::Relaxed);
        let backend = self.backend.clone();
        let clearing_deliberately = self.clearing_deliberately.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(u64::from(seconds))).await;
            if generation.load(Ordering::Relaxed) != baseline {
                return;
            }
            // Set right before the call: on Wayland (and possibly X11), an
            // explicit clear makes the selection empty the same way a
            // source application exiting does, and `handle_event` cannot
            // otherwise tell those apart — this flag is what lets it skip
            // `persist_after_owner_gone` for the specific OwnerGone event
            // this clear itself is about to cause, without also missing a
            // later, genuine owner-exit persistence should still handle.
            clearing_deliberately.store(true, Ordering::Relaxed);
            if let Err(e) = backend.clear(selection).await {
                clearing_deliberately.store(false, Ordering::Relaxed);
                debug!(error = %e, "clearing clipboard after recall failed");
            }
        });
    }

    /// Query history for IPC clients.
    pub fn query(&self, filter: &QueryFilter) -> Result<Vec<Entry>> {
        self.db.query(filter)
    }

    /// Toggle pin state and notify sync.
    pub async fn set_pinned(&self, id: i64, pinned: bool) -> Result<()> {
        self.db.set_pinned(id, pinned)?;
        self.stamp_local(id)?;
        self.bump();
        self.sync
            .on_event(SyncEvent::PinChanged { id, pinned })
            .await;
        Ok(())
    }

    /// Delete an entry (tombstone + blob cleanup) and notify sync.
    pub async fn delete(&self, id: i64) -> Result<()> {
        let entry = self.db.get(id)?;
        if entry.deleted {
            return Err(Error::NotFound(id));
        }
        self.db.tombstone(id, unix_now())?;
        self.stamp_local(id)?;
        self.collect_garbage()?;
        self.bump();
        self.sync
            .on_event(SyncEvent::EntryDeleted {
                id,
                content_hash: entry.content_hash,
            })
            .await;
        Ok(())
    }

    /// Undo a deletion while the tombstone is still within
    /// `UNDO_GRACE_SECS`; afterwards the row is gone and this is `NotFound`.
    pub async fn restore(&self, id: i64) -> Result<Entry> {
        // With sync on, the row outlives the undo window (without its
        // payloads); past the window it is gone as far as undo is concerned.
        match self.db.deleted_at(id)? {
            Some(at) if at == 0 || at < unix_now() - UNDO_GRACE_SECS => {
                return Err(Error::NotFound(id));
            }
            _ => {}
        }
        self.db.restore(id)?;
        self.stamp_local(id)?;
        self.bump();
        let entry = self.db.get(id)?;
        self.sync
            .on_event(SyncEvent::EntryUpserted(entry.clone()))
            .await;
        Ok(entry)
    }

    /// Stamp a row with the next local Lamport value, making its current
    /// state this device's newest change (SYNC-03).
    fn stamp_local(&self, id: i64) -> Result<()> {
        let lamport = self.db.tick_lamport()?;
        self.db.stamp(id, lamport, &self.device_id)
    }

    /// The change feed for a sync peer (SYNC-03): rows changed after
    /// `since`, oldest first, as records carrying their payloads. A reply
    /// stops at `limit` records and at what one v3 frame can carry (at most
    /// `MAX_FDS_PER_FRAME` payloads, `SYNC_REPLY_BYTES` of payload data,
    /// though a single larger entry still goes out on its own). Returns the
    /// records, the cursor to resume from, and whether more rows follow.
    pub fn sync_changes(
        &self,
        since: SyncCursor,
        limit: usize,
        scope: SyncScope,
    ) -> Result<(Vec<SyncRecord>, SyncCursor, bool)> {
        let limit = match limit {
            0 => SYNC_DEFAULT_LIMIT,
            n => n.min(SYNC_MAX_LIMIT),
        };
        let mut cursor = since;
        let mut records = Vec::new();
        let (mut payload_count, mut bytes) = (0usize, 0usize);
        'feed: loop {
            let rows = self.db.changes_since(cursor.seq, 64)?;
            if rows.is_empty() {
                break;
            }
            for (seq, entry) in rows {
                if records.len() >= limit {
                    break 'feed;
                }
                let here = SyncCursor { seq };
                if !scope.includes(entry.kind, entry.pinned, entry.deleted) {
                    cursor = here;
                    continue;
                }
                let payloads = if entry.deleted {
                    Vec::new()
                } else {
                    self.load_payloads(entry.id)?
                };
                // The receiver checks the hash against the payloads as sent.
                // Captures and imports store formats in the order they were
                // hashed in; a multi-format `panora-cli store` in another
                // order cannot be verified, so it stays on this device.
                if !entry.deleted && payload_hash(&payloads) != entry.content_hash {
                    debug!(
                        id = entry.id,
                        "payload order does not match the hash; not synced"
                    );
                    cursor = here;
                    continue;
                }
                if payloads.len() > MAX_FDS_PER_FRAME {
                    warn!(
                        id = entry.id,
                        formats = payloads.len(),
                        "too many formats to sync"
                    );
                    cursor = here;
                    continue;
                }
                let size: usize = payloads.iter().map(|p| p.data.len()).sum();
                if size > SYNC_MAX_ENTRY_BYTES {
                    warn!(id = entry.id, size, "entry too large to sync");
                    cursor = here;
                    continue;
                }
                if !records.is_empty()
                    && (payload_count + payloads.len() > MAX_FDS_PER_FRAME
                        || bytes + size > SYNC_REPLY_BYTES)
                {
                    break 'feed;
                }
                payload_count += payloads.len();
                bytes += size;
                records.push(SyncRecord {
                    content_hash: entry.content_hash,
                    selection: entry.selection,
                    source_app: entry.source_app,
                    created_at: entry.created_at,
                    last_seen_at: entry.last_seen_at,
                    pinned: entry.pinned,
                    deleted: entry.deleted,
                    device_id: entry.device_id,
                    lamport: entry.lamport,
                    payloads,
                });
                cursor = here;
            }
        }
        let more = !self.db.changes_since(cursor.seq, 1)?.is_empty();
        Ok((records, cursor, more))
    }

    /// Apply records from another device (SYNC-03), each one on its own by
    /// last-writer-wins. Returns how many were applied, ignored (lost to
    /// the local state, nothing to act on, or refused by the privacy gate
    /// or size limit the way a capture would be) and rejected (malformed).
    pub async fn sync_apply(&self, records: Vec<SyncRecord>) -> Result<(usize, usize, usize)> {
        let (mut applied, mut ignored, mut rejected) = (0, 0, 0);
        for record in records {
            match self.apply_record(record).await? {
                Applied::Yes => applied += 1,
                Applied::Ignored => ignored += 1,
                Applied::Rejected => rejected += 1,
            }
        }
        if applied > 0 {
            self.bump();
        }
        Ok((applied, ignored, rejected))
    }

    async fn apply_record(&self, record: SyncRecord) -> Result<Applied> {
        let remote = (record.lamport, record.device_id.as_str());
        if record.device_id.is_empty() || !(1..LAMPORT_CEILING).contains(&record.lamport) {
            return Ok(Applied::Rejected);
        }
        if !record.deleted
            && (record.payloads.is_empty() || payload_hash(&record.payloads) != record.content_hash)
        {
            return Ok(Applied::Rejected);
        }
        self.db.observe_lamport(record.lamport)?;
        let local = self.db.find(&record.content_hash, record.selection)?;
        if let Some(local) = &local {
            if !lww_wins(remote, (local.lamport, local.device_id.as_str())) {
                return Ok(Applied::Ignored);
            }
        }

        if record.deleted {
            let Some(local) = local else {
                // Nothing here to delete.
                return Ok(Applied::Ignored);
            };
            if !local.deleted {
                self.db.tombstone(local.id, unix_now())?;
                self.collect_garbage()?;
            }
            self.db.stamp(local.id, record.lamport, &record.device_id)?;
            self.sync
                .on_event(SyncEvent::EntryDeleted {
                    id: local.id,
                    content_hash: local.content_hash,
                })
                .await;
            return Ok(Applied::Yes);
        }

        if let Some(local) = local.as_ref().filter(|l| !l.deleted) {
            // Same content already here: take over pin state and times.
            self.db.apply_remote_state(
                local.id,
                record.pinned,
                record.created_at,
                record.last_seen_at,
                record.lamport,
                &record.device_id,
            )?;
            return Ok(Applied::Yes);
        }

        // New here, or deleted here earlier than the remote state: record it
        // through the same path and checks as a copy made on this machine.
        let limit = self.config().history.max_mime_bytes;
        if record.payloads.iter().any(|p| p.data.len() > limit) {
            return Ok(Applied::Ignored);
        }
        let data = ClipboardData {
            selection: record.selection,
            offered_mimes: record.payloads.iter().map(|p| p.mime.clone()).collect(),
            payloads: record.payloads,
            source_app: record.source_app,
        };
        let verdict = self
            .privacy
            .read()
            .map(|p| p.evaluate(&data))
            .unwrap_or(panora_core::privacy::Verdict::RejectPrivateMode);
        if !verdict.is_allowed() || !self.content_allowed(&data) {
            debug!(?verdict, "synced entry refused by privacy policy");
            return Ok(Applied::Ignored);
        }
        let entry = self.store(data).await?;
        self.db
            .set_times(entry.id, record.created_at, record.last_seen_at)?;
        self.db.apply_remote_state(
            entry.id,
            record.pinned,
            record.created_at,
            record.last_seen_at,
            record.lamport,
            &record.device_id,
        )?;
        Ok(Applied::Yes)
    }

    /// Record content a client handed over (`panora-cli store`) as if it
    /// had been copied. The privacy gate, size limit and lock state apply
    /// exactly as for a capture; `copy` also offers the entry on the
    /// clipboard.
    pub async fn store_external(&self, mut data: ClipboardData, copy: bool) -> Result<Entry> {
        if self.locked() {
            return Err(Error::PrivacyRejected);
        }
        data.payloads.retain(|payload| !payload.data.is_empty());
        if data.payloads.is_empty() {
            return Err(Error::Backend("nothing to store".into()));
        }
        let limit = self.config().history.max_mime_bytes;
        if let Some(payload) = data.payloads.iter().find(|p| p.data.len() > limit) {
            return Err(Error::TooLarge {
                size: payload.data.len(),
                limit,
            });
        }
        if data.offered_mimes.is_empty() {
            data.offered_mimes = data.payloads.iter().map(|p| p.mime.clone()).collect();
        }
        let verdict = self
            .privacy
            .read()
            .map(|p| p.evaluate(&data))
            .unwrap_or(panora_core::privacy::Verdict::RejectPrivateMode);
        if !verdict.is_allowed() {
            debug!(?verdict, "stored content rejected by privacy policy");
            return Err(Error::PrivacyRejected);
        }
        if !self.content_allowed(&data) {
            return Err(Error::PrivacyRejected);
        }
        let entry = self.store(data).await?;
        if copy {
            self.offer_entry(&entry).await?;
            self.remember_last_stored(Selection::Clipboard, entry.id);
        }
        Ok(entry)
    }

    /// Clear the history, keeping pinned entries. Returns the number cleared.
    pub async fn clear(&self) -> Result<usize> {
        let ids = self.db.clear_all()?;
        let n = ids.len();
        self.collect_garbage()?;
        self.bump();
        for id in ids {
            self.sync
                .on_event(SyncEvent::EntryDeleted {
                    id,
                    content_hash: String::new(),
                })
                .await;
        }
        Ok(n)
    }
}

/// The privacy engine a configuration describes: exclusions plus the
/// content filters. The patterns were validated with the configuration, so
/// a compile failure here can only mean a bug; falling back to no filters
/// keeps capture working rather than dropping everything.
fn privacy_engine_for(config: &Config) -> PrivacyEngine {
    let filters = match ContentFilters::from_config(&config.privacy) {
        Ok(filters) => filters,
        Err(e) => {
            warn!(error = %e, "content filters unusable; capturing without them");
            ContentFilters::default()
        }
    };
    PrivacyEngine::new(&config.privacy.excluded_apps)
        .with_filters(filters)
        .with_excluded_titles(&config.privacy.excluded_window_titles)
}

/// Characters of text indexed for search beyond the preview.
const FULL_TEXT_INDEX_CHARS: usize = 64 * 1024;

/// The text to index beyond the preview, when there is any: text-like
/// entries longer than the preview, capped at `FULL_TEXT_INDEX_CHARS`.
fn index_text(text: &str, kind: ContentKind) -> Option<String> {
    if !matches!(
        kind,
        ContentKind::Text | ContentKind::RichText | ContentKind::Link
    ) {
        return None;
    }
    let trimmed = text.trim();
    if trimmed.chars().count() <= PREVIEW_LEN {
        return None;
    }
    Some(trimmed.chars().take(FULL_TEXT_INDEX_CHARS).collect())
}

/// Current Unix time in seconds.
pub fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Position of a MIME type in `WANTED_MIMES` (unknown types sort last).
fn mime_rank(mime: &str) -> usize {
    WANTED_MIMES
        .iter()
        .position(|w| mime.eq_ignore_ascii_case(w))
        .unwrap_or(WANTED_MIMES.len())
}

/// The offered MIME types we capture, in our preference order, without
/// duplicates. Only one plain-text flavour is read: `text/plain` and
/// `UTF8_STRING` carry the same bytes and would double the blob work.
fn wanted_order(offered: &[String]) -> Vec<String> {
    let mut chosen: Vec<String> = offered
        .iter()
        .filter(|m| mime_rank(m) < WANTED_MIMES.len())
        .cloned()
        .collect();
    chosen.sort_by_key(|m| mime_rank(m));
    chosen.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    let mut have_text = false;
    chosen.retain(|m| {
        let is_plain = m.starts_with("text/plain") || m == "UTF8_STRING";
        if is_plain && have_text {
            return false;
        }
        if is_plain {
            have_text = true;
        }
        true
    });
    chosen
}

/// Build a preview string: text content truncated, or a kind label.
fn make_preview(text: &str, data: &ClipboardData) -> String {
    let t = text.trim();
    if !t.is_empty() {
        let mut s: String = t.chars().take(PREVIEW_LEN).collect();
        if t.chars().count() > PREVIEW_LEN {
            s.push('…');
        }
        return s;
    }
    match data.classify() {
        ContentKind::Image => "[image]".to_string(),
        ContentKind::FileList | ContentKind::Link => {
            let uris = data
                .payload_for("text/uri-list")
                .map(|p| uri_list_preview(&p.data))
                .unwrap_or_default();
            if uris.is_empty() {
                "[link]".to_string()
            } else {
                uris
            }
        }
        ContentKind::RichText => data
            .payload_for("text/html")
            .map(|p| strip_html(&String::from_utf8_lossy(&p.data)))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "[rich text]".to_string()),
        ContentKind::Color => "[color]".to_string(),
        other => format!("[{}]", other.as_str()),
    }
}

/// Human preview for a `text/uri-list` payload: file names, one per line.
fn uri_list_preview(data: &[u8]) -> String {
    let text = String::from_utf8_lossy(data);
    let names: Vec<String> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .take(20)
        .map(|uri| {
            let path = uri.strip_prefix("file://").unwrap_or(uri);
            let name = path
                .trim_end_matches('/')
                .rsplit('/')
                .next()
                .unwrap_or(path);
            percent_decode(name)
        })
        .collect();
    let mut out = names.join("\n");
    if out.chars().count() > PREVIEW_LEN {
        out = out.chars().take(PREVIEW_LEN).collect();
        out.push('…');
    }
    out
}

/// Decode `%XX` escapes (file names in URIs) without a dependency. Works on
/// bytes only: slicing the `&str` could land inside a multibyte character.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = (bytes[i + 1] as char).to_digit(16);
            let lo = (bytes[i + 2] as char).to_digit(16);
            if let (Some(hi), Some(lo)) = (hi, lo) {
                out.push((hi * 16 + lo) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Two TARGETS lists describe the same owner when they hold the same set
/// of names (order may differ between reads).
fn same_targets(a: &[String], b: &[String]) -> bool {
    let mut a: Vec<&str> = a.iter().map(String::as_str).collect();
    let mut b: Vec<&str> = b.iter().map(String::as_str).collect();
    a.sort_unstable();
    a.dedup();
    b.sort_unstable();
    b.dedup();
    a == b
}

/// Very small tag stripper for HTML-only clipboard content previews.
fn strip_html(html: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    let mut last_space = true;
    for ch in html.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if in_tag => {}
            c if c.is_whitespace() => {
                if !last_space {
                    out.push(' ');
                    last_space = true;
                }
            }
            c => {
                out.push(c);
                last_space = false;
            }
        }
        if out.chars().count() >= PREVIEW_LEN {
            out.push('…');
            break;
        }
    }
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use panora_core::backend::MockBackend;
    use panora_core::storage::{Cipher, MasterKey};
    use panora_core::sync::NoopSync;

    fn test_daemon(dir: &tempfile::TempDir) -> (Daemon, Arc<MockBackend>) {
        test_daemon_as(dir, "test-device")
    }

    fn test_daemon_as(dir: &tempfile::TempDir, device: &str) -> (Daemon, Arc<MockBackend>) {
        let backend = Arc::new(MockBackend::new());
        let cipher = Cipher::new(&MasterKey::generate());
        let db = Database::open(dir.path().join("history.db"), cipher).unwrap();
        let blobs = BlobStore::open(
            dir.path().join("blobs"),
            Cipher::new(&MasterKey::generate()),
        )
        .unwrap();
        // NOTE: db and blobs use different keys here only because each
        // generates its own; production wires ONE master key into both.
        let config = Config::default();
        let daemon = Daemon::new(
            backend.clone(),
            db,
            blobs,
            config,
            Arc::new(NoopSync),
            device.into(),
        );
        (daemon, backend)
    }

    // --- SYNC-03: the change feed and last-writer-wins apply ---

    async fn put_text(daemon: &Daemon, text: &str) -> Entry {
        daemon
            .store_external(
                ClipboardData {
                    selection: Selection::Clipboard,
                    payloads: vec![MimePayload::new("text/plain", text.as_bytes())],
                    offered_mimes: vec!["text/plain".into()],
                    source_app: Some("editor".into()),
                },
                false,
            )
            .await
            .unwrap()
    }

    fn feed(daemon: &Daemon) -> Vec<SyncRecord> {
        daemon
            .sync_changes(SyncCursor::default(), 0, SyncScope::default())
            .unwrap()
            .0
    }

    /// Everything `from` changed goes to `to`, the way a sync client would.
    async fn push(from: &Daemon, to: &Daemon) -> (usize, usize, usize) {
        to.sync_apply(feed(from)).await.unwrap()
    }

    fn visible(daemon: &Daemon) -> Vec<(String, bool)> {
        let mut rows: Vec<(String, bool)> = daemon
            .query(&QueryFilter::recent(100))
            .unwrap()
            .into_iter()
            .map(|e| (e.preview, e.pinned))
            .collect();
        rows.sort();
        rows
    }

    #[tokio::test]
    async fn synced_entries_keep_their_origin_and_payloads() {
        let (dir_a, dir_b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let (a, _) = test_daemon_as(&dir_a, "aaaa");
        let (b, _) = test_daemon_as(&dir_b, "bbbb");
        let original = put_text(&a, "from laptop").await;

        assert_eq!(push(&a, &b).await, (1, 0, 0));
        let copy =
            b.db.find(&original.content_hash, Selection::Clipboard)
                .unwrap()
                .unwrap();
        assert_eq!(copy.preview, "from laptop");
        assert_eq!(copy.device_id, "aaaa");
        assert_eq!(copy.lamport, original.lamport);
        assert_eq!(copy.last_seen_at, original.last_seen_at);
        assert_eq!(
            b.load_payloads(copy.id).unwrap(),
            vec![MimePayload::new("text/plain", "from laptop")]
        );
        // Applying the same records again changes nothing.
        assert_eq!(push(&a, &b).await, (0, 1, 0));
        // The receiver's clock moved past what it saw.
        let local = put_text(&b, "on desktop").await;
        assert!(local.lamport > original.lamport);
    }

    #[tokio::test]
    async fn pins_and_deletes_converge_by_last_writer() {
        let (dir_a, dir_b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let (a, _) = test_daemon_as(&dir_a, "aaaa");
        let (b, _) = test_daemon_as(&dir_b, "bbbb");
        let entry = put_text(&a, "shared note").await;
        put_text(&a, "doomed").await;
        push(&a, &b).await;

        // A pins one entry, B deletes the other; each learns the other's change.
        a.set_pinned(entry.id, true).await.unwrap();
        let doomed_on_b = b
            .query(&QueryFilter::recent(10))
            .unwrap()
            .into_iter()
            .find(|e| e.preview == "doomed")
            .unwrap();
        b.delete(doomed_on_b.id).await.unwrap();
        push(&a, &b).await;
        push(&b, &a).await;
        assert_eq!(visible(&a), vec![("shared note".to_string(), true)]);
        assert_eq!(visible(&a), visible(&b));

        // A stale state (an older unpinned version) never undoes a newer one.
        let stale = feed(&a)
            .into_iter()
            .find(|r| r.content_hash == entry.content_hash)
            .map(|r| SyncRecord {
                pinned: false,
                lamport: 1,
                ..r
            })
            .unwrap();
        assert_eq!(b.sync_apply(vec![stale]).await.unwrap(), (0, 1, 0));
        assert_eq!(visible(&b), vec![("shared note".to_string(), true)]);
    }

    #[tokio::test]
    async fn malformed_or_hostile_records_are_rejected() {
        let (dir_a, dir_b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let (a, _) = test_daemon_as(&dir_a, "aaaa");
        let (b, _) = test_daemon_as(&dir_b, "bbbb");
        put_text(&a, "genuine").await;
        let good = feed(&a).remove(0);

        let forged = SyncRecord {
            payloads: vec![MimePayload::new("text/plain", "forged")],
            ..good.clone()
        };
        let empty = SyncRecord {
            payloads: Vec::new(),
            ..good.clone()
        };
        let runaway = SyncRecord {
            lamport: i64::MAX,
            ..good.clone()
        };
        let anonymous = SyncRecord {
            device_id: String::new(),
            ..good.clone()
        };
        assert_eq!(
            b.sync_apply(vec![forged, empty, runaway, anonymous])
                .await
                .unwrap(),
            (0, 0, 4)
        );
        assert!(visible(&b).is_empty());
        assert_eq!(
            b.db.tick_lamport().unwrap(),
            1,
            "rejected records do not move the clock"
        );
    }

    #[tokio::test]
    async fn synced_entries_pass_the_privacy_gate() {
        let (dir_a, dir_b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let (a, _) = test_daemon_as(&dir_a, "aaaa");
        let (b, _) = test_daemon_as(&dir_b, "bbbb");
        put_text(&a, "from a password manager").await;
        let mut record = feed(&a).remove(0);
        record.source_app = Some("KeePassXC".into());
        assert_eq!(b.sync_apply(vec![record]).await.unwrap(), (0, 1, 0));

        b.set_private_mode(true);
        put_text(&a, "while private").await;
        let (applied, ignored, _) = push(&a, &b).await;
        assert_eq!((applied, ignored), (0, 2));
        assert!(visible(&b).is_empty());
    }

    #[tokio::test]
    async fn sensitive_entries_never_leave_and_scope_narrows_the_feed() {
        let dir = tempfile::tempdir().unwrap();
        let (a, _) = test_daemon_as(&dir, "aaaa");
        put_text(&a, "AKIAIOSFODNN7EXAMPLE").await;
        let pinned = put_text(&a, "keep me").await;
        a.set_pinned(pinned.id, true).await.unwrap();
        put_text(&a, "loose").await;

        let texts = |records: Vec<SyncRecord>| -> Vec<String> {
            records
                .iter()
                .map(|r| String::from_utf8_lossy(&r.payloads[0].data).into_owned())
                .collect()
        };
        let all = texts(feed(&a));
        assert_eq!(all.len(), 2, "{all:?}");
        assert!(!all.iter().any(|t| t.starts_with("AKIA")));
        let (only_pinned, _, _) = a
            .sync_changes(
                SyncCursor::default(),
                0,
                SyncScope {
                    pinned_only: true,
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(texts(only_pinned), vec!["keep me".to_string()]);
    }

    #[tokio::test]
    async fn the_feed_pages_without_losing_rows() {
        let dir = tempfile::tempdir().unwrap();
        let (a, _) = test_daemon_as(&dir, "aaaa");
        for i in 0..5 {
            put_text(&a, &format!("row {i}")).await;
        }
        let mut cursor = SyncCursor::default();
        let mut seen = Vec::new();
        loop {
            let (records, next, more) = a.sync_changes(cursor, 2, SyncScope::default()).unwrap();
            assert!(records.len() <= 2);
            seen.extend(records.into_iter().map(|r| r.lamport));
            cursor = next;
            if !more {
                break;
            }
        }
        assert_eq!(seen.len(), 5);
        assert!(seen.windows(2).all(|w| w[0] < w[1]));
    }

    fn text_event() -> ClipboardEvent {
        ClipboardEvent::changed(
            Selection::Clipboard,
            vec!["text/plain".into()],
            Some("test".into()),
        )
    }

    async fn offer_text(backend: &MockBackend, text: &str) {
        backend
            .offer(
                Selection::Clipboard,
                ClipboardData {
                    selection: Selection::Clipboard,
                    payloads: vec![MimePayload::new("text/plain", text.as_bytes())],
                    offered_mimes: vec!["text/plain".into()],
                    source_app: Some("test".into()),
                },
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn captures_and_recalls_text() {
        let dir = tempfile::tempdir().unwrap();
        let (daemon, backend) = test_daemon(&dir);
        offer_text(&backend, "merhaba panora").await;
        daemon.handle_event(text_event()).await;

        let entries = daemon.query(&QueryFilter::recent(10)).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].preview, "merhaba panora");
        let rev = daemon.revision();

        // Recall puts it back on the (mock) clipboard.
        let outcome = daemon
            .recall(entries[0].id, false, None, Selection::Clipboard)
            .await
            .unwrap();
        assert!(!outcome.pasted);
        let back = backend
            .read(Selection::Clipboard, "text/plain")
            .await
            .unwrap();
        assert_eq!(back, b"merhaba panora");
        assert!(daemon.revision() > rev);
    }

    #[tokio::test]
    async fn recall_with_paste_reports_unsupported_backend() {
        let dir = tempfile::tempdir().unwrap();
        let (daemon, backend) = test_daemon(&dir);
        offer_text(&backend, "paste me").await;
        daemon.handle_event(text_event()).await;
        let id = daemon.query(&QueryFilter::recent(1)).unwrap()[0].id;
        let outcome = daemon
            .recall(id, true, None, Selection::Clipboard)
            .await
            .unwrap();
        assert!(!outcome.pasted, "mock backend has no synthetic paste");
    }

    #[tokio::test]
    async fn recall_to_primary_does_not_touch_clipboard() {
        // CAP-10: recall to PRIMARY (middle-click paste) leaves whatever is
        // on CLIPBOARD alone, and never sends a paste keystroke (there is
        // no keyboard shortcut for a PRIMARY paste).
        let dir = tempfile::tempdir().unwrap();
        let (daemon, backend) = test_daemon(&dir);
        offer_text(&backend, "middle-click me").await;
        daemon.handle_event(text_event()).await;
        let id = daemon.query(&QueryFilter::recent(1)).unwrap()[0].id;

        let outcome = daemon
            .recall(id, true, None, Selection::Primary)
            .await
            .unwrap();
        assert!(!outcome.pasted, "paste is meaningless for PRIMARY");
        assert_eq!(
            backend
                .read(Selection::Primary, "text/plain")
                .await
                .unwrap(),
            b"middle-click me"
        );
        // The original capture is still what CLIPBOARD holds.
        assert_eq!(
            backend
                .read(Selection::Clipboard, "text/plain")
                .await
                .unwrap(),
            b"middle-click me"
        );
    }

    #[tokio::test]
    async fn clipboard_clears_itself_after_a_recall() {
        // CAP-07: with clear_clipboard_after_seconds set, a recall clears
        // the live clipboard after the delay — but only if nothing else was
        // put there in the meantime. Real time, not paused: the clear runs
        // on a detached `tokio::spawn`ed task that a paused/advanced clock
        // cannot reliably drive to completion from the test task alone.
        let dir = tempfile::tempdir().unwrap();
        let (daemon, backend) = test_daemon(&dir);
        let mut config = daemon.config();
        config.privacy.clear_clipboard_after_seconds = 1;
        daemon.apply_config(config).unwrap();

        offer_text(&backend, "temporary secret").await;
        daemon.handle_event(text_event()).await;
        let id = daemon.query(&QueryFilter::recent(1)).unwrap()[0].id;
        daemon
            .recall(id, false, None, Selection::Clipboard)
            .await
            .unwrap();
        assert!(!backend
            .read_targets(Selection::Clipboard)
            .await
            .unwrap()
            .is_empty());

        tokio::time::sleep(std::time::Duration::from_millis(1300)).await;
        assert!(
            backend
                .read_targets(Selection::Clipboard)
                .await
                .unwrap()
                .is_empty(),
            "clipboard should have been cleared"
        );
    }

    #[tokio::test]
    async fn deliberate_clear_is_not_undone_by_persistence() {
        // A real backend reports an emptied selection the same way whether
        // a source application exited or CAP-07 just cleared it on
        // purpose — `handle_event`'s OwnerGone branch cannot otherwise
        // tell those apart, and persistence exists specifically to refill
        // an emptied clipboard. Caught with a headless-sway run of
        // scripts/wayland-e2e.sh: the clear fired, then panod immediately
        // re-offered the very entry it had just cleared.
        let dir = tempfile::tempdir().unwrap();
        let (daemon, backend) = test_daemon(&dir);
        let mut config = daemon.config();
        config.privacy.clear_clipboard_after_seconds = 1;
        daemon.apply_config(config).unwrap();

        offer_text(&backend, "temporary secret").await;
        daemon.handle_event(text_event()).await;
        let id = daemon.query(&QueryFilter::recent(1)).unwrap()[0].id;
        daemon
            .recall(id, false, None, Selection::Clipboard)
            .await
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(1300)).await;
        assert!(backend
            .read_targets(Selection::Clipboard)
            .await
            .unwrap()
            .is_empty());

        // The backend would report exactly this next, on a real compositor.
        daemon
            .handle_event(ClipboardEvent::owner_gone(Selection::Clipboard))
            .await;
        assert!(
            backend
                .read_targets(Selection::Clipboard)
                .await
                .unwrap()
                .is_empty(),
            "persistence must not undo a deliberate clear"
        );
    }

    #[tokio::test]
    async fn clipboard_clear_is_cancelled_by_a_newer_copy() {
        let dir = tempfile::tempdir().unwrap();
        let (daemon, backend) = test_daemon(&dir);
        let mut config = daemon.config();
        config.privacy.clear_clipboard_after_seconds = 1;
        daemon.apply_config(config).unwrap();

        offer_text(&backend, "temporary secret").await;
        daemon.handle_event(text_event()).await;
        let id = daemon.query(&QueryFilter::recent(1)).unwrap()[0].id;
        daemon
            .recall(id, false, None, Selection::Clipboard)
            .await
            .unwrap();

        // Something else lands on the clipboard before the timer fires —
        // through handle_event, the only realistic way the daemon learns
        // of an external change (see the comment on `capture_generation`).
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        offer_text(&backend, "someone else's copy").await;
        daemon.handle_event(text_event()).await;

        tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
        assert_eq!(
            backend
                .read(Selection::Clipboard, "text/plain")
                .await
                .unwrap(),
            b"someone else's copy",
            "a newer copy must survive the stale clear timer"
        );
    }

    #[tokio::test]
    async fn clear_clipboard_after_seconds_zero_disables_it() {
        let dir = tempfile::tempdir().unwrap();
        let (daemon, backend) = test_daemon(&dir);
        offer_text(&backend, "not cleared").await;
        daemon.handle_event(text_event()).await;
        let id = daemon.query(&QueryFilter::recent(1)).unwrap()[0].id;
        daemon
            .recall(id, false, None, Selection::Clipboard)
            .await
            .unwrap();
        // The default is 0 (disabled); recall must not have scheduled a
        // background task that could later clear it out from under us.
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        assert!(!backend
            .read_targets(Selection::Clipboard)
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn recall_can_be_limited_to_plain_text() {
        let dir = tempfile::tempdir().unwrap();
        let (daemon, backend) = test_daemon(&dir);
        backend
            .offer(
                Selection::Clipboard,
                ClipboardData {
                    selection: Selection::Clipboard,
                    payloads: vec![
                        MimePayload::new("text/plain;charset=utf-8", "plain"),
                        MimePayload::new("text/html", "<b>plain</b>"),
                    ],
                    offered_mimes: vec!["text/plain;charset=utf-8".into(), "text/html".into()],
                    source_app: None,
                },
            )
            .await
            .unwrap();
        daemon
            .handle_event(ClipboardEvent::changed(
                Selection::Clipboard,
                vec!["text/plain;charset=utf-8".into(), "text/html".into()],
                None,
            ))
            .await;
        let id = daemon.query(&QueryFilter::recent(1)).unwrap()[0].id;

        daemon
            .recall(id, false, Some("text/plain"), Selection::Clipboard)
            .await
            .unwrap();
        let offered = backend.read_targets(Selection::Clipboard).await.unwrap();
        assert_eq!(offered, vec!["text/plain;charset=utf-8".to_string()]);

        daemon
            .recall(id, false, None, Selection::Clipboard)
            .await
            .unwrap();
        assert_eq!(
            backend
                .read_targets(Selection::Clipboard)
                .await
                .unwrap()
                .len(),
            2
        );
        assert!(daemon
            .recall(id, false, Some("image/png"), Selection::Clipboard)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn secret_flag_content_never_stored() {
        let dir = tempfile::tempdir().unwrap();
        let (daemon, backend) = test_daemon(&dir);
        offer_text(&backend, "super-secret-password").await;
        daemon
            .handle_event(ClipboardEvent::changed(
                Selection::Clipboard,
                vec!["x-kde-passwordManagerHint".into(), "text/plain".into()],
                Some("keepassxc".into()),
            ))
            .await;
        assert_eq!(daemon.query(&QueryFilter::recent(10)).unwrap().len(), 0);
    }

    #[tokio::test]
    async fn excluded_app_content_never_stored() {
        let dir = tempfile::tempdir().unwrap();
        let (daemon, backend) = test_daemon(&dir);
        offer_text(&backend, "bitwarden-secret").await;
        daemon
            .handle_event(ClipboardEvent::changed(
                Selection::Clipboard,
                vec!["text/plain".into()],
                Some("Bitwarden".into()),
            ))
            .await;
        assert_eq!(daemon.query(&QueryFilter::recent(10)).unwrap().len(), 0);
    }

    #[tokio::test]
    async fn reloaded_exclusions_apply_live() {
        let dir = tempfile::tempdir().unwrap();
        let (daemon, backend) = test_daemon(&dir);
        offer_text(&backend, "bank data").await;
        let event = ClipboardEvent::changed(
            Selection::Clipboard,
            vec!["text/plain".into()],
            Some("MyBank".into()),
        );
        daemon.handle_event(event.clone()).await;
        assert_eq!(daemon.query(&QueryFilter::recent(10)).unwrap().len(), 1);

        let mut config = Config::default();
        config.privacy.excluded_apps.push("mybank".into());
        daemon.set_private_mode(true);
        daemon.apply_config(config).unwrap();
        assert!(daemon.private_mode(), "reload keeps private mode");
        daemon.set_private_mode(false);

        offer_text(&backend, "more bank data").await;
        daemon.handle_event(event).await;
        assert_eq!(daemon.query(&QueryFilter::recent(10)).unwrap().len(), 1);
    }

    #[tokio::test]
    async fn private_mode_pauses_recording() {
        let dir = tempfile::tempdir().unwrap();
        let (daemon, backend) = test_daemon(&dir);
        daemon.set_private_mode(true);
        offer_text(&backend, "not recorded").await;
        daemon.handle_event(text_event()).await;
        assert_eq!(daemon.query(&QueryFilter::recent(10)).unwrap().len(), 0);
        daemon.set_private_mode(false);
        offer_text(&backend, "recorded").await;
        daemon.handle_event(text_event()).await;
        assert_eq!(daemon.query(&QueryFilter::recent(10)).unwrap().len(), 1);
    }

    #[tokio::test]
    async fn oversized_payload_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let (daemon, backend) = test_daemon(&dir);
        let big = vec![b'x'; 11 * 1024 * 1024]; // 11 MiB > 10 MiB limit
        backend
            .offer(
                Selection::Clipboard,
                ClipboardData {
                    selection: Selection::Clipboard,
                    payloads: vec![MimePayload::new("text/plain", big)],
                    offered_mimes: vec!["text/plain".into()],
                    source_app: Some("test".into()),
                },
            )
            .await
            .unwrap();
        daemon.handle_event(text_event()).await;
        assert_eq!(daemon.query(&QueryFilter::recent(10)).unwrap().len(), 0);
    }

    #[tokio::test]
    async fn pin_and_delete_flow() {
        let dir = tempfile::tempdir().unwrap();
        let (daemon, backend) = test_daemon(&dir);
        offer_text(&backend, "pin me").await;
        daemon.handle_event(text_event()).await;
        let id = daemon.query(&QueryFilter::recent(10)).unwrap()[0].id;

        daemon.set_pinned(id, true).await.unwrap();
        assert!(daemon.db().get(id).unwrap().pinned);

        daemon.delete(id).await.unwrap();
        assert_eq!(daemon.query(&QueryFilter::recent(10)).unwrap().len(), 0);
        assert!(daemon
            .recall(id, false, None, Selection::Clipboard)
            .await
            .is_err());
        assert!(daemon.delete(id).await.is_err());
    }

    #[tokio::test]
    async fn deleting_one_entry_keeps_shared_blob() {
        let dir = tempfile::tempdir().unwrap();
        let (daemon, backend) = test_daemon(&dir);
        // Two entries sharing the same text/plain bytes but different HTML.
        for html in ["<b>x</b>", "<i>x</i>"] {
            backend
                .offer(
                    Selection::Clipboard,
                    ClipboardData {
                        selection: Selection::Clipboard,
                        payloads: vec![
                            MimePayload::new("text/plain", "x"),
                            MimePayload::new("text/html", html),
                        ],
                        offered_mimes: vec!["text/plain".into(), "text/html".into()],
                        source_app: None,
                    },
                )
                .await
                .unwrap();
            daemon
                .handle_event(ClipboardEvent::changed(
                    Selection::Clipboard,
                    vec!["text/plain".into(), "text/html".into()],
                    None,
                ))
                .await;
        }
        let entries = daemon.query(&QueryFilter::recent(10)).unwrap();
        assert_eq!(entries.len(), 2);
        daemon.delete(entries[0].id).await.unwrap();
        let payloads = daemon.load_payloads(entries[1].id).unwrap();
        assert_eq!(payloads.len(), 2);
        assert_eq!(payloads[0].mime, "text/plain");
        assert_eq!(payloads[0].data, b"x");
    }

    #[tokio::test]
    async fn eviction_removes_blobs_from_disk() {
        let dir = tempfile::tempdir().unwrap();
        let (daemon, backend) = test_daemon(&dir);
        let mut config = Config::default();
        config.history.max_entries = 2;
        daemon.apply_config(config).unwrap();
        for i in 0..5 {
            offer_text(&backend, &format!("entry number {i}")).await;
            daemon.handle_event(text_event()).await;
        }
        assert_eq!(daemon.db().count().unwrap(), 2);
        let files = count_files(&dir.path().join("blobs"));
        assert_eq!(files, 2, "evicted payloads must not linger on disk");
    }

    fn count_files(dir: &std::path::Path) -> usize {
        let mut n = 0;
        if let Ok(rd) = std::fs::read_dir(dir) {
            for e in rd.flatten() {
                if e.path().is_dir() {
                    n += count_files(&e.path());
                } else {
                    n += 1;
                }
            }
        }
        n
    }

    #[tokio::test]
    async fn owner_gone_reoffers_last_entry() {
        let dir = tempfile::tempdir().unwrap();
        let (daemon, backend) = test_daemon(&dir);
        offer_text(&backend, "keep me around").await;
        daemon.handle_event(text_event()).await;
        // Simulate the source app exiting: the mock now offers nothing.
        backend
            .offer(
                Selection::Clipboard,
                ClipboardData {
                    selection: Selection::Clipboard,
                    payloads: vec![],
                    offered_mimes: vec![],
                    source_app: None,
                },
            )
            .await
            .unwrap();
        daemon
            .handle_event(ClipboardEvent::owner_gone(Selection::Clipboard))
            .await;
        let back = backend
            .read(Selection::Clipboard, "text/plain")
            .await
            .unwrap();
        assert_eq!(back, b"keep me around");
    }

    #[tokio::test]
    async fn primary_ignored_unless_configured() {
        let dir = tempfile::tempdir().unwrap();
        let (daemon, backend) = test_daemon(&dir);
        backend
            .offer(
                Selection::Primary,
                ClipboardData {
                    selection: Selection::Primary,
                    payloads: vec![MimePayload::new("text/plain", "selected")],
                    offered_mimes: vec!["text/plain".into()],
                    source_app: None,
                },
            )
            .await
            .unwrap();
        let event = ClipboardEvent::changed(Selection::Primary, vec!["text/plain".into()], None);
        daemon.handle_event(event.clone()).await;
        assert_eq!(daemon.db().count().unwrap(), 0);
        let mut config = Config::default();
        config.history.record_primary = true;
        daemon.apply_config(config).unwrap();
        daemon.handle_event(event).await;
        assert_eq!(daemon.db().count().unwrap(), 1);
    }

    /// A real PNG of the given size, so the thumbnail path decodes something.
    fn png_of(width: u32, height: u32) -> Vec<u8> {
        let image = image::RgbaImage::from_fn(width, height, |x, y| {
            image::Rgba([(x % 256) as u8, (y % 256) as u8, 90, 255])
        });
        let mut out = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image)
            .write_to(&mut out, image::ImageFormat::Png)
            .unwrap();
        out.into_inner()
    }

    fn png_size(bytes: &[u8]) -> (u32, u32) {
        let decoded = image::load_from_memory(bytes).unwrap();
        (decoded.width(), decoded.height())
    }

    #[tokio::test]
    async fn image_entries_get_a_thumbnail_that_never_reaches_the_clipboard() {
        let dir = tempfile::tempdir().unwrap();
        let (daemon, backend) = test_daemon(&dir);
        let big = png_of(1600, 400);
        backend
            .offer(
                Selection::Clipboard,
                ClipboardData {
                    selection: Selection::Clipboard,
                    payloads: vec![MimePayload::new("image/png", big.clone())],
                    offered_mimes: vec!["image/png".into()],
                    source_app: None,
                },
            )
            .await
            .unwrap();
        daemon
            .handle_event(ClipboardEvent::changed(
                Selection::Clipboard,
                vec!["image/png".into()],
                None,
            ))
            .await;
        let id = daemon.query(&QueryFilter::recent(1)).unwrap()[0].id;

        let thumbnail = daemon.load_thumbnail(id).unwrap().expect("thumbnail");
        assert_eq!(thumbnail.mime, THUMBNAIL_MIME);
        let (w, h) = png_size(&thumbnail.data);
        assert_eq!((w, h), (320, 80), "shrunk to the longest side, aspect kept");
        assert!(thumbnail.data.len() < big.len());

        // Clients asking for the row image get just the thumbnail; the full
        // payloads and the clipboard never see it.
        let row = daemon.thumbnail_or_full(id).await.unwrap();
        assert_eq!(row.len(), 1);
        assert_eq!(row[0].mime, THUMBNAIL_MIME);
        let full = daemon.load_payloads(id).unwrap();
        assert_eq!(full.len(), 1);
        assert_eq!(full[0].mime, "image/png");
        assert_eq!(full[0].data, big);
        daemon
            .recall(id, false, None, Selection::Clipboard)
            .await
            .unwrap();
        let offered = backend.read_targets(Selection::Clipboard).await.unwrap();
        assert_eq!(offered, vec!["image/png".to_string()]);

        // Small images are kept as they are; text entries answer with their
        // payloads.
        backend
            .offer(
                Selection::Clipboard,
                ClipboardData {
                    selection: Selection::Clipboard,
                    payloads: vec![MimePayload::new("image/png", png_of(64, 48))],
                    offered_mimes: vec!["image/png".into()],
                    source_app: None,
                },
            )
            .await
            .unwrap();
        daemon
            .handle_event(ClipboardEvent::changed(
                Selection::Clipboard,
                vec!["image/png".into()],
                None,
            ))
            .await;
        let small_id = daemon.query(&QueryFilter::recent(1)).unwrap()[0].id;
        let small = daemon.load_thumbnail(small_id).unwrap().unwrap();
        assert_eq!(png_size(&small.data), (64, 48));
        offer_text(&backend, "just text").await;
        daemon.handle_event(text_event()).await;
        let text_id = daemon.query(&QueryFilter::recent(1)).unwrap()[0].id;
        assert!(daemon.load_thumbnail(text_id).unwrap().is_none());
        assert_eq!(
            daemon.thumbnail_or_full(text_id).await.unwrap()[0].data,
            b"just text"
        );
    }

    #[tokio::test]
    async fn thumbnails_are_made_on_demand_for_older_entries() {
        let dir = tempfile::tempdir().unwrap();
        let (daemon, backend) = test_daemon(&dir);
        backend
            .offer(
                Selection::Clipboard,
                ClipboardData {
                    selection: Selection::Clipboard,
                    payloads: vec![MimePayload::new("image/png", png_of(900, 900))],
                    offered_mimes: vec!["image/png".into()],
                    source_app: None,
                },
            )
            .await
            .unwrap();
        daemon
            .handle_event(ClipboardEvent::changed(
                Selection::Clipboard,
                vec!["image/png".into()],
                None,
            ))
            .await;
        let id = daemon.query(&QueryFilter::recent(1)).unwrap()[0].id;
        // Pretend the entry predates thumbnails: drop the blob reference.
        let thumb_ref = daemon
            .db()
            .blobs_of(id)
            .unwrap()
            .into_iter()
            .find(|(m, _)| m == THUMBNAIL_MIME)
            .unwrap()
            .1;
        daemon.db().detach_blob(id, THUMBNAIL_MIME).unwrap();
        daemon.blobs().remove(&thumb_ref).unwrap();
        assert!(daemon.load_thumbnail(id).unwrap().is_none());

        let row = daemon.thumbnail_or_full(id).await.unwrap();
        assert_eq!(row[0].mime, THUMBNAIL_MIME);
        assert_eq!(png_size(&row[0].data), (320, 320));
        assert!(
            daemon.load_thumbnail(id).unwrap().is_some(),
            "kept for next time"
        );
    }

    #[tokio::test]
    async fn search_finds_words_beyond_the_preview() {
        let dir = tempfile::tempdir().unwrap();
        let (daemon, backend) = test_daemon(&dir);
        let long = format!("{} zebrafinal", "filler word ".repeat(120));
        assert!(long.chars().count() > PREVIEW_LEN);
        offer_text(&backend, &long).await;
        daemon.handle_event(text_event()).await;
        let found = daemon
            .query(&QueryFilter {
                search: Some("zebrafinal".into()),
                ..QueryFilter::recent(10)
            })
            .unwrap();
        assert_eq!(found.len(), 1);
        assert!(found[0].preview.ends_with('…'));

        // With indexing off, only the preview is searchable.
        let mut config = Config::default();
        config.history.index_full_text = false;
        daemon.apply_config(config).unwrap();
        let other = format!("{} okapifinal", "other filler ".repeat(120));
        offer_text(&backend, &other).await;
        daemon.handle_event(text_event()).await;
        assert!(daemon
            .query(&QueryFilter {
                search: Some("okapifinal".into()),
                ..QueryFilter::recent(10)
            })
            .unwrap()
            .is_empty());
        assert_eq!(
            daemon
                .query(&QueryFilter {
                    search: Some("other".into()),
                    ..QueryFilter::recent(10)
                })
                .unwrap()
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn excluded_window_titles_keep_the_copy_out() {
        let dir = tempfile::tempdir().unwrap();
        let (daemon, backend) = test_daemon(&dir);
        let mut config = Config::default();
        config.privacy.excluded_window_titles = vec!["Online Banking".into()];
        daemon.apply_config(config).unwrap();

        offer_text(&backend, "IBAN copied from the bank").await;
        daemon
            .handle_event(text_event().with_title(Some("Online banking — Firefox".into())))
            .await;
        assert!(daemon.query(&QueryFilter::recent(10)).unwrap().is_empty());

        offer_text(&backend, "a recipe").await;
        daemon
            .handle_event(text_event().with_title(Some("Recipes — Firefox".into())))
            .await;
        assert_eq!(daemon.query(&QueryFilter::recent(10)).unwrap().len(), 1);

        // No title known (plain Wayland): the gate stays open.
        offer_text(&backend, "another note").await;
        daemon.handle_event(text_event()).await;
        assert_eq!(daemon.query(&QueryFilter::recent(10)).unwrap().len(), 2);
    }

    #[tokio::test]
    async fn maintenance_removes_orphaned_blobs_only() {
        let dir = tempfile::tempdir().unwrap();
        let (daemon, backend) = test_daemon(&dir);
        offer_text(&backend, "kept because a row points at it").await;
        daemon.handle_event(text_event()).await;
        let entry = daemon.query(&QueryFilter::recent(1)).unwrap().remove(0);
        let orphan = daemon.blobs.put(b"nobody references this").unwrap();
        assert!(daemon.blobs.exists(&orphan));
        assert_eq!(daemon.maintain().unwrap(), 1);
        assert!(!daemon.blobs.exists(&orphan));
        assert_eq!(
            daemon.load_payloads(entry.id).unwrap()[0].data,
            b"kept because a row points at it"
        );
        assert_eq!(daemon.maintain().unwrap(), 0);
    }

    #[tokio::test]
    async fn total_bytes_cap_is_applied_on_store() {
        let dir = tempfile::tempdir().unwrap();
        let (daemon, backend) = test_daemon(&dir);
        let mut config = Config::default();
        config.history.max_total_bytes = 60;
        daemon.apply_config(config).unwrap();
        for text in [
            "first entry of forty characters exactly!!",
            "second one is also quite long enough.",
            "short",
        ] {
            offer_text(&backend, text).await;
            daemon.handle_event(text_event()).await;
        }
        let previews: Vec<String> = daemon
            .query(&QueryFilter::recent(10))
            .unwrap()
            .into_iter()
            .map(|e| e.preview)
            .collect();
        assert_eq!(
            previews,
            vec!["short", "second one is also quite long enough."]
        );
    }

    #[tokio::test]
    async fn secrets_are_masked_flagged_and_still_recallable() {
        let dir = tempfile::tempdir().unwrap();
        let (daemon, backend) = test_daemon(&dir);
        offer_text(&backend, "AKIAIOSFODNN7EXAMPLE").await;
        daemon.handle_event(text_event()).await;
        let entry = daemon.query(&QueryFilter::recent(1)).unwrap().remove(0);
        assert!(entry.sensitive);
        assert!(
            entry.preview.starts_with(SENSITIVE_MASK),
            "{}",
            entry.preview
        );
        assert!(entry.preview.contains("API token"));
        assert!(
            daemon
                .query(&QueryFilter {
                    search: Some("EXAMPLE".into()),
                    ..QueryFilter::recent(10)
                })
                .unwrap()
                .is_empty(),
            "the secret itself is not searchable"
        );
        let payloads = daemon.load_payloads(entry.id).unwrap();
        assert_eq!(payloads[0].data, b"AKIAIOSFODNN7EXAMPLE");

        // `store` keeps the text visible but flags it.
        let mut config = Config::default();
        config.privacy.sensitive_policy = "store".into();
        daemon.apply_config(config).unwrap();
        offer_text(&backend, "ghp_A1b2C3d4E5f6G7h8I9j0K1l2M3n4O5p6Q7r8").await;
        daemon.handle_event(text_event()).await;
        let entry = daemon.query(&QueryFilter::recent(1)).unwrap().remove(0);
        assert!(entry.sensitive);
        assert!(entry.preview.starts_with("ghp_"));

        // `drop` records nothing.
        let mut config = Config::default();
        config.privacy.sensitive_policy = "drop".into();
        daemon.apply_config(config).unwrap();
        let before = daemon.query(&QueryFilter::recent(10)).unwrap().len();
        offer_text(&backend, "xoxb-000000000000-not-a-real-token-at-all-000").await;
        daemon.handle_event(text_event()).await;
        assert_eq!(
            daemon.query(&QueryFilter::recent(10)).unwrap().len(),
            before
        );

        // Ordinary text is untouched by any of this.
        offer_text(&backend, "a shopping list").await;
        daemon.handle_event(text_event()).await;
        let entry = daemon.query(&QueryFilter::recent(1)).unwrap().remove(0);
        assert!(!entry.sensitive);
        assert_eq!(entry.preview, "a shopping list");
    }

    #[tokio::test]
    async fn content_filters_apply_after_the_mime_gate() {
        let dir = tempfile::tempdir().unwrap();
        let (daemon, backend) = test_daemon(&dir);
        let mut config = Config::default();
        config.privacy.min_text_length = 3;
        config.privacy.ignore_patterns = vec!["^\\d{16}$".into()];
        config.privacy.capture_kinds = vec!["text".into()];
        daemon.apply_config(config).unwrap();
        for (text, kept) in [
            ("hello there", true),
            ("ab", false),
            ("1234567890123456", false),
            ("   ", false),
            ("https://example.org/x", false),
            ("#ff8800", false),
            ("plain enough", true),
        ] {
            offer_text(&backend, text).await;
            daemon.handle_event(text_event()).await;
            let stored = daemon
                .query(&QueryFilter::recent(10))
                .unwrap()
                .iter()
                .any(|e| e.preview == text.trim());
            assert_eq!(stored, kept, "{text:?}");
        }
        // The same filters guard the bridge and the CLI store path.
        assert!(matches!(
            daemon
                .store_external(
                    ClipboardData {
                        selection: Selection::Clipboard,
                        payloads: vec![MimePayload::new("text/plain", "ab")],
                        offered_mimes: vec![],
                        source_app: None,
                    },
                    false,
                )
                .await,
            Err(Error::PrivacyRejected)
        ));
    }

    #[tokio::test]
    async fn delete_can_be_undone_until_the_grace_period_ends() {
        let dir = tempfile::tempdir().unwrap();
        let (daemon, backend) = test_daemon(&dir);
        offer_text(&backend, "oops, deleted").await;
        daemon.handle_event(text_event()).await;
        let id = daemon.query(&QueryFilter::recent(1)).unwrap()[0].id;
        daemon.delete(id).await.unwrap();
        assert!(daemon.query(&QueryFilter::recent(10)).unwrap().is_empty());
        assert!(daemon.load_payloads(id).is_err());

        // Within the grace period the row and its blobs are still there.
        let restored = daemon.restore(id).await.unwrap();
        assert_eq!(restored.preview, "oops, deleted");
        assert_eq!(daemon.load_payloads(id).unwrap()[0].data, b"oops, deleted");
        let found = daemon
            .query(&QueryFilter {
                search: Some("oops".into()),
                ..QueryFilter::recent(10)
            })
            .unwrap();
        assert_eq!(found.len(), 1, "restored entries are searchable again");

        // Once the grace period has passed (simulated by purging everything
        // tombstoned "before the end of time"), the undo is refused and
        // nothing lingers on disk.
        daemon.delete(id).await.unwrap();
        daemon.purge_tombstoned_before(i64::MAX).unwrap();
        assert!(daemon.restore(id).await.is_err());
        assert_eq!(count_files(&dir.path().join("blobs")), 0);
    }

    #[tokio::test]
    async fn external_store_goes_through_the_privacy_gate() {
        let dir = tempfile::tempdir().unwrap();
        let (daemon, backend) = test_daemon(&dir);
        let data = |text: &str, app: Option<&str>| ClipboardData {
            selection: Selection::Clipboard,
            payloads: vec![MimePayload::new("text/plain", text.as_bytes())],
            offered_mimes: Vec::new(),
            source_app: app.map(str::to_string),
        };
        let entry = daemon
            .store_external(data("from a script", None), true)
            .await
            .unwrap();
        assert_eq!(entry.preview, "from a script");
        assert_eq!(
            backend
                .read(Selection::Clipboard, "text/plain")
                .await
                .unwrap(),
            b"from a script",
            "copy=true offers the content"
        );
        assert!(matches!(
            daemon
                .store_external(data("secret", Some("KeePassXC")), false)
                .await,
            Err(Error::PrivacyRejected)
        ));
        daemon.set_private_mode(true);
        assert!(daemon
            .store_external(data("while private", None), false)
            .await
            .is_err());
        daemon.set_private_mode(false);
        let mut config = Config::default();
        config.history.max_mime_bytes = 8;
        daemon.apply_config(config).unwrap();
        assert!(matches!(
            daemon
                .store_external(data("far too long for eight bytes", None), false)
                .await,
            Err(Error::TooLarge { .. })
        ));
        assert_eq!(daemon.db().count().unwrap(), 1);
    }

    #[tokio::test]
    async fn locked_session_pauses_recording_without_touching_private_mode() {
        let dir = tempfile::tempdir().unwrap();
        let (daemon, backend) = test_daemon(&dir);
        daemon.set_locked(true);
        assert!(daemon.status().unwrap().locked);
        assert!(!daemon.private_mode());
        offer_text(&backend, "typed on the lock screen").await;
        daemon.handle_event(text_event()).await;
        assert_eq!(daemon.db().count().unwrap(), 0);
        daemon.set_locked(false);
        assert!(!daemon.status().unwrap().locked);
        offer_text(&backend, "back at the desk").await;
        daemon.handle_event(text_event()).await;
        assert_eq!(daemon.db().count().unwrap(), 1);
    }

    /// Collects everything the daemon logs during a test.
    struct LogSink(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for LogSink {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn logs_never_carry_clipboard_content() {
        let dir = tempfile::tempdir().unwrap();
        let (daemon, backend) = test_daemon(&dir);
        let sink = Arc::new(Mutex::new(Vec::new()));
        let writer = sink.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE)
            .with_writer(move || LogSink(writer.clone()))
            .finish();
        let secret = "MARKER-7f3a9c-must-never-be-logged";
        {
            let _guard = tracing::subscriber::set_default(subscriber);
            offer_text(&backend, secret).await;
            daemon.handle_event(text_event()).await;
            // Paths that log at warn/debug: rejection, oversize, recall,
            // delete, an external store and a reload.
            daemon
                .handle_event(ClipboardEvent::changed(
                    Selection::Clipboard,
                    vec!["x-kde-passwordManagerHint".into(), "text/plain".into()],
                    Some("keepassxc".into()),
                ))
                .await;
            let mut config = Config::default();
            config.history.max_mime_bytes = 4;
            daemon.apply_config(config).unwrap();
            offer_text(&backend, &format!("{secret}-oversized")).await;
            daemon.handle_event(text_event()).await;
            daemon.apply_config(Config::default()).unwrap();
            let id = daemon.query(&QueryFilter::recent(1)).unwrap()[0].id;
            daemon
                .recall(id, true, None, Selection::Clipboard)
                .await
                .unwrap();
            let _ = daemon
                .store_external(
                    ClipboardData {
                        selection: Selection::Clipboard,
                        payloads: vec![MimePayload::new("text/plain", secret.as_bytes())],
                        offered_mimes: vec!["x-kde-passwordManagerHint".into()],
                        source_app: None,
                    },
                    false,
                )
                .await;
            daemon.delete(id).await.unwrap();
        }
        let logs = String::from_utf8_lossy(&sink.lock().unwrap()).into_owned();
        assert!(!logs.is_empty(), "the daemon must have logged something");
        assert!(
            !logs.contains("MARKER-7f3a9c"),
            "clipboard content leaked into the log:\n{logs}"
        );
    }

    /// Poll `condition` for up to two seconds.
    async fn eventually(mut condition: impl FnMut() -> bool) -> bool {
        for _ in 0..200 {
            if condition() {
                return true;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        condition()
    }

    #[tokio::test]
    async fn primary_watch_follows_config_reload() {
        let dir = tempfile::tempdir().unwrap();
        let (daemon, backend) = test_daemon(&dir);
        let daemon = std::rc::Rc::new(daemon);
        let (shutdown_tx, shutdown_rx) = mpsc::channel(1);
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let runner = daemon.clone();
                let loop_task =
                    tokio::task::spawn_local(async move { runner.run(shutdown_rx).await });
                assert!(eventually(|| backend.watching(Selection::Clipboard)).await);
                // Off by default: no PRIMARY watch is opened at all.
                assert!(!backend.watching(Selection::Primary));

                let primary = ClipboardData {
                    selection: Selection::Primary,
                    payloads: vec![MimePayload::new("text/plain", "selected text")],
                    offered_mimes: vec!["text/plain".into()],
                    source_app: None,
                };
                backend.offer(Selection::Primary, primary).await.unwrap();
                let event =
                    ClipboardEvent::changed(Selection::Primary, vec!["text/plain".into()], None);

                // Turning it on through a reload must open the watch live.
                let mut config = Config::default();
                config.history.record_primary = true;
                daemon.apply_config(config).unwrap();
                assert!(eventually(|| backend.watching(Selection::Primary)).await);
                backend.push_event(event.clone()).await;
                assert!(eventually(|| daemon.db().count().unwrap() == 1).await);

                // Turning it off drops the watch: later PRIMARY changes are
                // neither delivered nor stored.
                daemon.apply_config(Config::default()).unwrap();
                assert!(eventually(|| !backend.watching(Selection::Primary)).await);
                backend.push_event(event).await;
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                assert_eq!(daemon.db().count().unwrap(), 1);

                shutdown_tx.send(()).await.unwrap();
                loop_task.await.unwrap().unwrap();
            })
            .await;
    }

    #[tokio::test]
    async fn image_payload_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let (daemon, backend) = test_daemon(&dir);
        let png = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]; // PNG magic
        backend
            .offer(
                Selection::Clipboard,
                ClipboardData {
                    selection: Selection::Clipboard,
                    payloads: vec![MimePayload::new("image/png", png.clone())],
                    offered_mimes: vec!["image/png".into()],
                    source_app: Some("test".into()),
                },
            )
            .await
            .unwrap();
        daemon
            .handle_event(ClipboardEvent::changed(
                Selection::Clipboard,
                vec!["image/png".into()],
                Some("test".into()),
            ))
            .await;
        let entries = daemon.query(&QueryFilter::recent(10)).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].kind, ContentKind::Image);
        let payloads = daemon.load_payloads(entries[0].id).unwrap();
        assert_eq!(payloads[0].data, png);
        let status = daemon.status().unwrap();
        assert_eq!(status.entries, 1);
        assert_eq!(status.protocol, PROTOCOL_VERSION);
    }

    /// Mock that presents itself as the Wayland backend with a given
    /// compositor persistence verdict.
    struct WaylandMock(MockBackend, bool);

    #[async_trait::async_trait]
    impl ClipboardBackend for WaylandMock {
        fn name(&self) -> &'static str {
            "wayland"
        }
        fn capabilities(&self) -> panora_core::backend::Capabilities {
            panora_core::backend::Capabilities {
                persist: self.1,
                source_app: false,
                ..self.0.capabilities()
            }
        }
        async fn watch(&self, s: Selection) -> Result<mpsc::Receiver<ClipboardEvent>> {
            self.0.watch(s).await
        }
        async fn read_targets(&self, s: Selection) -> Result<Vec<String>> {
            self.0.read_targets(s).await
        }
        async fn read(&self, s: Selection, m: &str) -> Result<Vec<u8>> {
            self.0.read(s, m).await
        }
        async fn offer(&self, s: Selection, d: ClipboardData) -> Result<()> {
            self.0.offer(s, d).await
        }
    }

    /// Store one text entry through `backend`, then simulate the owner
    /// leaving; returns what the (mock) clipboard holds afterwards.
    async fn after_owner_gone(
        dir: &tempfile::TempDir,
        backend: Arc<WaylandMock>,
        policy: &str,
    ) -> Option<Vec<u8>> {
        let cipher = Cipher::new(&MasterKey::generate());
        let db = Database::open(dir.path().join("history.db"), cipher).unwrap();
        let blobs = BlobStore::open(
            dir.path().join("blobs"),
            Cipher::new(&MasterKey::generate()),
        )
        .unwrap();
        let mut config = Config::default();
        config.history.persist_on_wayland = policy.into();
        let daemon = Daemon::new(
            backend.clone(),
            db,
            blobs,
            config,
            Arc::new(NoopSync),
            "wl".into(),
        );
        offer_text(&backend.0, "keep me on wayland").await;
        daemon.handle_event(text_event()).await;
        // The source exits: the compositor reports an empty selection.
        backend
            .0
            .offer(
                Selection::Clipboard,
                ClipboardData {
                    selection: Selection::Clipboard,
                    payloads: vec![],
                    offered_mimes: vec![],
                    source_app: None,
                },
            )
            .await
            .unwrap();
        daemon
            .handle_event(ClipboardEvent::owner_gone(Selection::Clipboard))
            .await;
        backend
            .0
            .read(Selection::Clipboard, "text/plain")
            .await
            .ok()
    }

    #[tokio::test]
    async fn wayland_persistence_follows_the_compositor_and_the_setting() {
        let expected = b"keep me on wayland".to_vec();
        for (compositor_drops, policy, restored) in [
            (true, "auto", true),   // wlroots: re-offer
            (false, "auto", false), // Mutter/KWin keep it themselves: leave it
            (false, "always", true),
            (true, "never", false),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let backend = Arc::new(WaylandMock(MockBackend::new(), compositor_drops));
            let held = after_owner_gone(&dir, backend, policy).await;
            assert_eq!(
                held.is_some_and(|bytes| bytes == expected),
                restored,
                "drops={compositor_drops} policy={policy}"
            );
        }
    }

    /// Mock that claims to depend on the GNOME bridge (GNOME <= 47 Wayland).
    struct BridgeMock(MockBackend);

    #[async_trait::async_trait]
    impl ClipboardBackend for BridgeMock {
        fn name(&self) -> &'static str {
            "gnome-bridge"
        }
        fn capabilities(&self) -> panora_core::backend::Capabilities {
            panora_core::backend::Capabilities {
                needs_bridge: true,
                ..self.0.capabilities()
            }
        }
        async fn watch(&self, s: Selection) -> Result<mpsc::Receiver<ClipboardEvent>> {
            self.0.watch(s).await
        }
        async fn read_targets(&self, s: Selection) -> Result<Vec<String>> {
            self.0.read_targets(s).await
        }
        async fn read(&self, s: Selection, m: &str) -> Result<Vec<u8>> {
            self.0.read(s, m).await
        }
        async fn offer(&self, s: Selection, d: ClipboardData) -> Result<()> {
            self.0.offer(s, d).await
        }
    }

    fn bridge_push(text: &str, app: Option<&str>) -> ClipboardData {
        ClipboardData {
            selection: Selection::Clipboard,
            payloads: vec![MimePayload::new("text/plain;charset=utf-8", text)],
            offered_mimes: vec!["text/plain;charset=utf-8".into(), "text/plain".into()],
            source_app: app.map(str::to_string),
        }
    }

    #[tokio::test]
    async fn health_reports_a_missing_extension_only_when_capture_needs_it() {
        let dir = tempfile::tempdir().unwrap();
        let (daemon, _backend) = test_daemon(&dir);
        daemon.set_extension_present(false);
        assert!(
            daemon.status().unwrap().health.is_empty(),
            "a backend that captures on its own has no use for the extension"
        );

        let dir = tempfile::tempdir().unwrap();
        let daemon = Daemon::new(
            Arc::new(BridgeMock(MockBackend::new())),
            Database::open(
                dir.path().join("history.db"),
                Cipher::new(&MasterKey::generate()),
            )
            .unwrap(),
            BlobStore::open(
                dir.path().join("blobs"),
                Cipher::new(&MasterKey::generate()),
            )
            .unwrap(),
            Config::default(),
            Arc::new(NoopSync),
            "test-device".into(),
        );
        assert!(
            daemon.status().unwrap().health.is_empty(),
            "presence is assumed until the watcher reports"
        );
        let revision = daemon.revision();
        daemon.set_extension_present(false);
        let health = daemon.status().unwrap().health;
        assert_eq!(health.len(), 1);
        assert_eq!(health[0].code, health::EXTENSION_MISSING);
        assert!(health[0].message.contains(health::EXTENSION_UUID));
        assert!(daemon.revision() > revision, "clients poll the revision");
        daemon.set_extension_present(true);
        assert!(daemon.status().unwrap().health.is_empty());
    }

    #[tokio::test]
    async fn bridge_captures_and_ignores_the_echo_of_a_recall() {
        let dir = tempfile::tempdir().unwrap();
        let backend = Arc::new(BridgeMock(MockBackend::new()));
        let daemon = Daemon::new(
            backend.clone(),
            Database::open(
                dir.path().join("history.db"),
                Cipher::new(&MasterKey::generate()),
            )
            .unwrap(),
            BlobStore::open(
                dir.path().join("blobs"),
                Cipher::new(&MasterKey::generate()),
            )
            .unwrap(),
            Config::default(),
            Arc::new(NoopSync),
            "test-device".into(),
        );

        daemon
            .handle_gnome_data(bridge_push("first", Some("firefox.desktop")), None)
            .await;
        daemon
            .handle_gnome_data(bridge_push("second", Some("firefox.desktop")), None)
            .await;
        assert_eq!(daemon.db().count().unwrap(), 2);
        let first = daemon.query(&QueryFilter::recent(10)).unwrap()[1].id;

        // Recall "first": the extension sees the clipboard change and pushes
        // the very same bytes back. That must not create a third entry.
        daemon
            .recall(first, false, None, Selection::Clipboard)
            .await
            .unwrap();
        daemon
            .handle_gnome_data(bridge_push("first", None), None)
            .await;
        assert_eq!(daemon.db().count().unwrap(), 2);
        assert_eq!(daemon.query(&QueryFilter::recent(1)).unwrap()[0].id, first);

        // A genuinely new copy right after the recall is still recorded.
        daemon
            .handle_gnome_data(bridge_push("third", None), None)
            .await;
        assert_eq!(daemon.db().count().unwrap(), 3);

        // Password managers are filtered by their desktop id too.
        daemon
            .handle_gnome_data(
                bridge_push("hunter2", Some("org.keepassxc.KeePassXC.desktop")),
                None,
            )
            .await;
        assert_eq!(daemon.db().count().unwrap(), 3);
    }

    #[tokio::test]
    async fn gnome_bridge_ignored_when_native_backend_active() {
        let dir = tempfile::tempdir().unwrap();
        let (daemon, _backend) = test_daemon(&dir);
        daemon
            .handle_gnome_data(
                ClipboardData {
                    selection: Selection::Clipboard,
                    payloads: vec![MimePayload::new("text/plain", "from shell")],
                    offered_mimes: vec!["text/plain".into()],
                    source_app: None,
                },
                None,
            )
            .await;
        assert_eq!(daemon.db().count().unwrap(), 0);
    }

    #[test]
    fn wanted_order_prefers_one_text_flavour() {
        let offered = vec![
            "TIMESTAMP".to_string(),
            "UTF8_STRING".to_string(),
            "text/html".to_string(),
            "text/plain;charset=utf-8".to_string(),
            "text/plain".to_string(),
        ];
        assert_eq!(
            wanted_order(&offered),
            vec![
                "text/plain;charset=utf-8".to_string(),
                "text/html".to_string()
            ]
        );
    }

    #[test]
    fn previews_for_non_text_kinds() {
        let files = ClipboardData {
            selection: Selection::Clipboard,
            payloads: vec![MimePayload::new(
                "text/uri-list",
                "file:///home/u/Belgeler/rapor%20son.pdf\r\nfile:///tmp/x.png\r\n",
            )],
            offered_mimes: vec![
                "x-special/gnome-copied-files".into(),
                "text/uri-list".into(),
            ],
            source_app: None,
        };
        assert_eq!(make_preview("", &files), "rapor son.pdf\nx.png");

        let html = ClipboardData {
            selection: Selection::Clipboard,
            payloads: vec![MimePayload::new(
                "text/html",
                "<p>Hello <b>world</b></p>\n<p>again</p>",
            )],
            offered_mimes: vec!["text/html".into()],
            source_app: None,
        };
        assert_eq!(make_preview("", &html), "Hello world again");
        assert_eq!(percent_decode("a%2Fb%zz"), "a/b%zz");
        assert_eq!(percent_decode("50%aç.txt"), "50%aç.txt");
        assert_eq!(percent_decode("%C3%A7ay%"), "çay%");
        assert!(same_targets(
            &["a".into(), "b".into()],
            &["b".into(), "a".into(), "a".into()]
        ));
        assert!(!same_targets(&["a".into()], &["a".into(), "b".into()]));
    }

    #[tokio::test]
    async fn owner_gone_never_restores_a_rejected_change() {
        let dir = tempfile::tempdir().unwrap();
        let (daemon, backend) = test_daemon(&dir);
        offer_text(&backend, "public").await;
        daemon.handle_event(text_event()).await;
        // A later change that the privacy engine rejects (password manager)
        // clears the restore candidate: an exit or clear afterwards must not
        // put "public" back on the clipboard.
        daemon
            .handle_event(ClipboardEvent::changed(
                Selection::Clipboard,
                vec!["x-kde-passwordManagerHint".into(), "text/plain".into()],
                Some("keepassxc".into()),
            ))
            .await;
        backend
            .offer(
                Selection::Clipboard,
                ClipboardData {
                    selection: Selection::Clipboard,
                    payloads: vec![],
                    offered_mimes: vec![],
                    source_app: None,
                },
            )
            .await
            .unwrap();
        daemon
            .handle_event(ClipboardEvent::owner_gone(Selection::Clipboard))
            .await;
        assert!(backend
            .read(Selection::Clipboard, "text/plain")
            .await
            .is_err());
    }

    #[tokio::test]
    async fn capture_discarded_when_owner_changes_during_read() {
        let dir = tempfile::tempdir().unwrap();
        let (daemon, backend) = test_daemon(&dir);
        // The mock serves whatever was offered last; the event claims a
        // different TARGETS list than what the backend now reports.
        offer_text(&backend, "second owner").await;
        daemon
            .handle_event(ClipboardEvent::changed(
                Selection::Clipboard,
                vec!["text/plain".into(), "text/html".into()],
                None,
            ))
            .await;
        assert_eq!(daemon.db().count().unwrap(), 0);
    }

    // --- SEC-02: second-layer lock. `set_lock_password` itself needs a
    // real Secret Service (it wraps a backup copy of the live master key
    // for the keyring) and is covered by `tests/lock.rs` instead; these
    // seed `db.set_lock_secret` directly to exercise engage/unlock/idle
    // logic without one.

    fn seed_lock_secret(daemon: &Daemon, password: &str) {
        let secret = panora_core::lock::LockSecret::new(password).unwrap();
        daemon.db().set_lock_secret(Some(&secret)).unwrap();
    }

    #[tokio::test]
    async fn engage_lock_refuses_without_a_password_set() {
        let dir = tempfile::tempdir().unwrap();
        let (daemon, _backend) = test_daemon(&dir);
        assert!(!daemon.lock_password_set().unwrap());
        assert!(daemon.engage_lock().is_err());
        assert!(!daemon.is_app_locked());
    }

    #[tokio::test]
    async fn engage_and_unlock_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let (daemon, _backend) = test_daemon(&dir);
        seed_lock_secret(&daemon, "correct horse battery staple");

        daemon.engage_lock().unwrap();
        assert!(daemon.is_app_locked());

        assert!(daemon.unlock("wrong password").is_err());
        assert!(daemon.is_app_locked(), "a wrong password must not unlock");

        daemon.unlock("correct horse battery staple").unwrap();
        assert!(!daemon.is_app_locked());
    }

    #[tokio::test]
    async fn maybe_auto_lock_only_fires_once_idle_and_configured() {
        let dir = tempfile::tempdir().unwrap();
        let (daemon, _backend) = test_daemon(&dir);
        seed_lock_secret(&daemon, "idle timeout password");

        // Disabled (the default): never locks, no matter how idle.
        daemon.backdate_activity_for_test(10 * 60);
        daemon.maybe_auto_lock();
        assert!(!daemon.is_app_locked());

        // Enabled, but not idle long enough yet.
        {
            let mut config = daemon.config();
            config.privacy.lock_after_idle_minutes = 5;
            daemon.apply_config(config).unwrap();
        }
        daemon.backdate_activity_for_test(60); // 1 minute, under the 5 configured
        daemon.maybe_auto_lock();
        assert!(!daemon.is_app_locked());

        // Idle past the configured threshold: engages.
        daemon.backdate_activity_for_test(6 * 60);
        daemon.maybe_auto_lock();
        assert!(daemon.is_app_locked());
    }

    #[tokio::test]
    async fn maybe_auto_lock_does_nothing_without_a_password() {
        let dir = tempfile::tempdir().unwrap();
        let (daemon, _backend) = test_daemon(&dir);
        {
            let mut config = daemon.config();
            config.privacy.lock_after_idle_minutes = 1;
            daemon.apply_config(config).unwrap();
        }
        daemon.backdate_activity_for_test(120);
        daemon.maybe_auto_lock();
        assert!(!daemon.is_app_locked(), "nothing to lock with");
    }

    // --- CLI-03: encrypted export/import ---

    #[tokio::test]
    async fn export_then_import_into_an_empty_history_restores_the_same_records() {
        let source_dir = tempfile::tempdir().unwrap();
        let (source, source_backend) = test_daemon(&source_dir);
        offer_text(&source_backend, "first exported entry").await;
        source.handle_event(text_event()).await;
        offer_text(&source_backend, "second exported entry, pinned").await;
        source.handle_event(text_event()).await;
        let pinned_id = source.query(&QueryFilter::recent(1)).unwrap()[0].id;
        source.set_pinned(pinned_id, true).await.unwrap();
        let png = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        source_backend
            .offer(
                Selection::Clipboard,
                ClipboardData {
                    selection: Selection::Clipboard,
                    payloads: vec![MimePayload::new("image/png", png.clone())],
                    offered_mimes: vec!["image/png".into()],
                    source_app: Some("test".into()),
                },
            )
            .await
            .unwrap();
        source
            .handle_event(ClipboardEvent::changed(
                Selection::Clipboard,
                vec!["image/png".into()],
                Some("test".into()),
            ))
            .await;
        assert_eq!(source.db().count().unwrap(), 3);

        let archive = source.export("correct horse battery staple").unwrap();

        let dest_dir = tempfile::tempdir().unwrap();
        let (dest, _dest_backend) = test_daemon(&dest_dir);
        let imported = dest
            .import("correct horse battery staple", &archive)
            .await
            .unwrap();
        assert_eq!(imported, 3);
        assert_eq!(dest.db().count().unwrap(), 3);

        let by_preview = |d: &Daemon, preview: &str| -> Entry {
            d.query(&QueryFilter::recent(10))
                .unwrap()
                .into_iter()
                .find(|e| e.preview == preview)
                .unwrap()
        };
        let first = by_preview(&dest, "first exported entry");
        assert!(!first.pinned);
        assert_eq!(
            dest.load_payloads(first.id).unwrap()[0].data,
            b"first exported entry"
        );

        let second = by_preview(&dest, "second exported entry, pinned");
        assert!(second.pinned, "pinned state survives the round trip");

        let image = dest
            .query(&QueryFilter::recent(10))
            .unwrap()
            .into_iter()
            .find(|e| e.kind == ContentKind::Image)
            .unwrap();
        assert_eq!(dest.load_payloads(image.id).unwrap()[0].data, png);
    }

    #[tokio::test]
    async fn import_deduplicates_by_content_hash_like_a_live_capture() {
        let dir = tempfile::tempdir().unwrap();
        let (daemon, backend) = test_daemon(&dir);
        offer_text(&backend, "will be exported and re-imported").await;
        daemon.handle_event(text_event()).await;
        assert_eq!(daemon.db().count().unwrap(), 1);

        let archive = daemon.export("pw").unwrap();
        let imported = daemon.import("pw", &archive).await.unwrap();

        assert_eq!(imported, 1);
        assert_eq!(
            daemon.db().count().unwrap(),
            1,
            "importing an entry already present must not duplicate it"
        );
    }

    #[tokio::test]
    async fn import_rejects_the_wrong_passphrase() {
        let dir = tempfile::tempdir().unwrap();
        let (daemon, backend) = test_daemon(&dir);
        offer_text(&backend, "protected by a passphrase").await;
        daemon.handle_event(text_event()).await;
        let archive = daemon.export("right passphrase").unwrap();

        assert!(daemon.import("wrong passphrase", &archive).await.is_err());
    }
}

/// Property tests (QA-07): `percent_decode` and `wanted_order` both parse
/// or reorder data a remote clipboard owner controls (a URI list, the set
/// of MIME types a window offers), so they need to hold for input no unit
/// test happened to type, not just the handful of cases above.
#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    /// Percent-encode every byte as `%XX` (uppercase hex), the inverse of
    /// `percent_decode` for the escaped path.
    fn percent_encode_all(bytes: &[u8]) -> String {
        let mut out = String::with_capacity(bytes.len() * 3);
        for b in bytes {
            out.push_str(&format!("%{b:02X}"));
        }
        out
    }

    proptest! {
        /// Never panics on arbitrary text: a file name inside a
        /// `text/uri-list` payload is exactly the kind of untrusted string
        /// this decodes, and the byte-oriented loop must stay in bounds
        /// whatever `%`-runs or multi-byte characters it holds.
        #[test]
        fn percent_decode_never_panics(s in ".{0,200}") {
            let _ = percent_decode(&s);
        }

        /// A string with no `%` byte passes through unchanged.
        #[test]
        fn percent_decode_is_identity_without_percent(s in "[^%]{0,100}") {
            prop_assert_eq!(percent_decode(&s), s);
        }

        /// Fully percent-encoding a valid UTF-8 string and decoding it back
        /// recovers the original — the round trip the file-name-in-a-URI
        /// case actually relies on.
        #[test]
        fn percent_decode_roundtrips_encoded_utf8(s in ".{0,80}") {
            let encoded = percent_encode_all(s.as_bytes());
            prop_assert_eq!(percent_decode(&encoded), s);
        }
    }

    /// A handful of real offered types plus junk strings, so most runs mix
    /// recognised MIME types (to exercise ordering/dedup) with values the
    /// grammar does not know at all (to exercise the "unknown" filter).
    fn offered_mime_strategy() -> impl Strategy<Value = Vec<String>> {
        let known = prop_oneof![
            Just("text/plain;charset=utf-8".to_string()),
            Just("text/plain".to_string()),
            Just("TEXT/PLAIN".to_string()),
            Just("UTF8_STRING".to_string()),
            Just("text/html".to_string()),
            Just("image/png".to_string()),
            Just("text/uri-list".to_string()),
        ];
        let junk = "[a-zA-Z0-9/;=_-]{0,20}";
        prop::collection::vec(prop_oneof![known, junk.prop_map(String::from)], 0..15)
    }

    proptest! {
        /// Every entry `wanted_order` keeps is one it recognises: dropping
        /// unknown offered types is the whole point of the `mime_rank`
        /// filter, so nothing unranked should ever survive.
        #[test]
        fn wanted_order_only_keeps_known_mimes(offered in offered_mime_strategy()) {
            for m in wanted_order(&offered) {
                prop_assert!(mime_rank(&m) < WANTED_MIMES.len(), "unranked mime kept: {m}");
            }
        }

        /// The result is sorted by preference and holds no duplicate (by
        /// the same case-insensitive comparison `dedup_by` uses) — a
        /// capture loop that reads the same rank twice would waste work
        /// fetching one flavour redundantly.
        #[test]
        fn wanted_order_is_sorted_and_deduped(offered in offered_mime_strategy()) {
            let chosen = wanted_order(&offered);
            for pair in chosen.windows(2) {
                prop_assert!(mime_rank(&pair[0]) <= mime_rank(&pair[1]));
                prop_assert!(!pair[0].eq_ignore_ascii_case(&pair[1]));
            }
        }

        /// At most one plain-text flavour is kept, whichever of
        /// `text/plain;charset=utf-8`, `text/plain` or `UTF8_STRING` was
        /// offered — they carry the same bytes, so capturing more than one
        /// would only double the blob work.
        #[test]
        fn wanted_order_keeps_at_most_one_text_flavour(offered in offered_mime_strategy()) {
            let is_plain = |m: &str| m.starts_with("text/plain") || m == "UTF8_STRING";
            let plain_count = wanted_order(&offered).iter().filter(|m| is_plain(m)).count();
            prop_assert!(plain_count <= 1);
        }

        /// Idempotent: the chosen list is already in the shape `wanted_order`
        /// produces, so filtering it again must be a no-op.
        #[test]
        fn wanted_order_is_idempotent(offered in offered_mime_strategy()) {
            let once = wanted_order(&offered);
            let twice = wanted_order(&once);
            prop_assert_eq!(once, twice);
        }
    }
}
