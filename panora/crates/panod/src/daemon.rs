// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! Daemon core: wires backend events through the privacy engine into
//! encrypted storage, and notifies the sync provider (ADR 0002).

use panora_core::backend::{ClipboardBackend, ClipboardEvent, EventKind};
use panora_core::config::Config;
use panora_core::error::{Error, Result};
use panora_core::ipc::{CapabilityData, StatusData, PROTOCOL_VERSION};
use panora_core::model::{ClipboardData, ContentKind, Entry, MimePayload, Selection};
use panora_core::privacy::PrivacyEngine;
use panora_core::storage::{BlobStore, Database, QueryFilter};
use panora_core::sync::{SyncEvent, SyncProvider};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
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

/// Delay between putting data on the clipboard and synthesizing Ctrl+V, so
/// the popup has time to close and focus returns to the target window.
const PASTE_DELAY: std::time::Duration = std::time::Duration::from_millis(160);

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
    /// Entry stored from the most recent change of each selection; the only
    /// content the persistence path may re-offer.
    last_stored: Mutex<HashMap<Selection, i64>>,
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
        let privacy = PrivacyEngine::new(&config.privacy.excluded_apps);
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
            last_stored: Mutex::new(HashMap::new()),
        }
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
        self.revision.fetch_add(1, Ordering::Relaxed);
    }

    /// Replace the configuration (from `config.toml`) without a restart.
    /// Private mode is user state, not configuration, so it is preserved.
    pub fn apply_config(&self, config: Config) -> Result<()> {
        config.validate()?;
        let private_now = self.private_mode();
        let privacy = PrivacyEngine::new(&config.privacy.excluded_apps);
        privacy.set_private_mode(private_now);
        if let Ok(mut slot) = self.privacy.write() {
            *slot = privacy;
        }
        if let Ok(mut slot) = self.config.write() {
            *slot = config;
        }
        info!("configuration reloaded");
        self.collect_garbage()?;
        self.bump();
        Ok(())
    }

    /// Re-read `config.toml` and apply it.
    pub fn reload_config(&self) -> Result<()> {
        self.apply_config(Config::load()?)
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
            },
        })
    }

    /// Run the capture loop until the shutdown channel closes.
    pub async fn run(&self, mut shutdown: mpsc::Receiver<()>) -> Result<()> {
        let mut rx = self.backend.watch(Selection::Clipboard).await?;
        let record_primary = self.config().history.record_primary;
        let mut primary_rx = if record_primary && self.backend.capabilities().primary {
            Some(self.backend.watch(Selection::Primary).await?)
        } else {
            None
        };

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

    /// Process one clipboard change event: privacy gate first, then
    /// payload read, then encrypted storage, then sync notification.
    pub async fn handle_event(&self, event: ClipboardEvent) {
        if event.kind == EventKind::OwnerGone {
            self.persist_after_owner_gone(event.selection).await;
            return;
        }
        // Whatever happens below, a change that is not stored must not be
        // "restored" later by the persistence path.
        self.forget_last_stored(event.selection);
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
    pub async fn handle_gnome_data(&self, data: ClipboardData) {
        if !self.backend.capabilities().needs_bridge {
            // A native data-control backend already captured this change
            // with every format; storing the single bridge payload too would
            // create a second, poorer entry.
            debug!("GNOME bridge payload ignored: native backend is active");
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

        if let Err(e) = self.store(data).await {
            warn!(error = %e, "failed to store GNOME bridge clipboard event");
        }
    }

    /// Persist captured data: dedup by content hash, encrypt payloads
    /// into the blob store, index metadata + preview in SQLite/FTS5.
    pub async fn store(&self, data: ClipboardData) -> Result<Entry> {
        let kind = data.classify();
        let text = data.text().unwrap_or_default();
        let preview = make_preview(&text, &data);
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
        let lamport = self.db.max_lamport()? + 1;

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
        )?;

        // Store payloads as encrypted blobs and attach references.
        for p in &data.payloads {
            let blob_ref = self.blobs.put(&p.data)?;
            self.db.attach_blob(id, &p.mime, &blob_ref)?;
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
        // v1 has no sync peer to replay tombstones to, so they are purged
        // right away; the column stays for the future sync module.
        let orphaned = self.db.purge_tombstones(i64::MAX)?;
        for blob_ref in orphaned {
            if let Err(e) = self.blobs.remove(&blob_ref) {
                warn!(blob = %blob_ref, error = %e, "blob cleanup failed");
            }
        }
        if !evicted.is_empty() {
            debug!(count = evicted.len(), "evicted entries by retention policy");
        }
        Ok(evicted.len())
    }

    /// Load an entry's payloads back from the blob store.
    pub fn load_payloads(&self, entry_id: i64) -> Result<Vec<MimePayload>> {
        let entry = self.db.get(entry_id)?;
        if entry.deleted {
            return Err(Error::NotFound(entry_id));
        }
        let mut out = Vec::new();
        for (mime, blob_ref) in self.db.blobs_of(entry_id)? {
            let data = self.blobs.get(&blob_ref)?;
            out.push(MimePayload { mime, data });
        }
        // Keep the daemon's preference order so clients and backends see a
        // stable "primary" format first.
        out.sort_by_key(|p| mime_rank(&p.mime));
        Ok(out)
    }

    /// Offer an entry on the CLIPBOARD selection. Entries captured from
    /// PRIMARY are recalled to the clipboard too: that is what the user
    /// pastes with Ctrl+V.
    async fn offer_entry(&self, entry: &Entry) -> Result<()> {
        let payloads = self.load_payloads(entry.id)?;
        if payloads.is_empty() {
            return Err(Error::NotFound(entry.id));
        }
        let data = ClipboardData {
            selection: Selection::Clipboard,
            offered_mimes: payloads.iter().map(|p| p.mime.clone()).collect(),
            source_app: Some("panora".into()),
            payloads,
        };
        self.backend.offer(Selection::Clipboard, data).await
    }

    /// Put a history entry back on the clipboard, then optionally paste it.
    pub async fn recall(&self, entry_id: i64, paste: bool) -> Result<RecallOutcome> {
        let entry = self.db.get(entry_id)?;
        if entry.deleted {
            return Err(Error::NotFound(entry_id));
        }
        self.offer_entry(&entry).await?;
        self.db.touch(entry_id, unix_now())?;
        self.bump();
        if !paste {
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

    /// Query history for IPC clients.
    pub fn query(&self, filter: &QueryFilter) -> Result<Vec<Entry>> {
        self.db.query(filter)
    }

    /// Toggle pin state and notify sync.
    pub async fn set_pinned(&self, id: i64, pinned: bool) -> Result<()> {
        self.db.set_pinned(id, pinned)?;
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
        self.db.tombstone(id)?;
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
            "test-device".into(),
        );
        (daemon, backend)
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
        let outcome = daemon.recall(entries[0].id, false).await.unwrap();
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
        let outcome = daemon.recall(id, true).await.unwrap();
        assert!(!outcome.pasted, "mock backend has no synthetic paste");
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
        assert!(daemon.recall(id, false).await.is_err());
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

    #[tokio::test]
    async fn gnome_bridge_ignored_when_native_backend_active() {
        let dir = tempfile::tempdir().unwrap();
        let (daemon, _backend) = test_daemon(&dir);
        daemon
            .handle_gnome_data(ClipboardData {
                selection: Selection::Clipboard,
                payloads: vec![MimePayload::new("text/plain", "from shell")],
                offered_mimes: vec!["text/plain".into()],
                source_app: None,
            })
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
}
