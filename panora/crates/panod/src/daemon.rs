// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! Daemon core: wires backend events through the privacy engine into
//! encrypted storage, and notifies the sync provider (ADR 0002).

use panora_core::backend::{ClipboardBackend, ClipboardEvent};
use panora_core::config::Config;
use panora_core::error::Result;
use panora_core::model::{ClipboardData, Entry, MimePayload, Selection};
use panora_core::privacy::PrivacyEngine;
use panora_core::storage::{BlobStore, Database, QueryFilter};
use panora_core::sync::{SyncEvent, SyncProvider};
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

/// MIME types we always try to capture, in preference order.
const WANTED_MIMES: &[&str] = &[
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

/// The daemon: owns backend, privacy engine, storage and sync provider.
pub struct Daemon {
    backend: Arc<dyn ClipboardBackend>,
    privacy: PrivacyEngine,
    db: Database,
    blobs: BlobStore,
    sync: Arc<dyn SyncProvider>,
    config: Config,
    device_id: String,
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
            privacy,
            db,
            blobs,
            sync,
            config,
            device_id,
        }
    }

    /// Access the privacy engine (for private-mode toggles over IPC).
    pub fn privacy(&self) -> &PrivacyEngine {
        &self.privacy
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

    /// Access the config.
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Run the capture loop until the shutdown channel closes.
    pub async fn run(&self, mut shutdown: mpsc::Receiver<()>) -> Result<()> {
        let mut rx = self.backend.watch(Selection::Clipboard).await?;
        let mut primary_rx =
            if self.config.history.record_primary && self.backend.capabilities().primary {
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
                        None => return Ok(()),
                    }
                }
                ev = async {
                    match primary_rx.as_mut() {
                        Some(r) => r.recv().await,
                        None => std::future::pending().await,
                    }
                } => {
                    if let Some(e) = ev {
                        self.handle_event(e).await;
                    }
                }
            }
        }
    }

    /// Process one clipboard change event: privacy gate first, then
    /// payload read, then encrypted storage, then sync notification.
    pub async fn handle_event(&self, event: ClipboardEvent) {
        // Gate 1+2: evaluate policy using ONLY the offered MIME list and
        // the source app name. Payloads have not been read yet (ADR 0003).
        let probe = ClipboardData {
            selection: event.selection,
            payloads: Vec::new(),
            offered_mimes: event.offered_mimes.clone(),
            source_app: event.source_app.clone(),
        };
        let verdict = self.privacy.evaluate(&probe);
        if !verdict.is_allowed() {
            debug!(?verdict, "content rejected by privacy policy");
            return;
        }

        // Read payloads for the MIME types we support.
        let mut payloads: Vec<MimePayload> = Vec::new();
        for mime in &event.offered_mimes {
            let wanted = WANTED_MIMES.iter().any(|w| mime.eq_ignore_ascii_case(w));
            if !wanted {
                continue;
            }
            match self.backend.read(event.selection, mime).await {
                Ok(bytes) => {
                    if bytes.is_empty() {
                        continue;
                    }
                    if bytes.len() > self.config.history.max_mime_bytes {
                        warn!(
                            mime,
                            size = bytes.len(),
                            limit = self.config.history.max_mime_bytes,
                            "payload exceeds size limit, skipping"
                        );
                        continue;
                    }
                    payloads.push(MimePayload {
                        mime: mime.clone(),
                        data: bytes,
                    });
                }
                Err(e) => {
                    debug!(mime, error = %e, "failed to read payload");
                }
            }
        }
        if payloads.is_empty() {
            return;
        }

        let data = ClipboardData {
            selection: event.selection,
            payloads,
            offered_mimes: event.offered_mimes.clone(),
            source_app: event.source_app.clone(),
        };
        if let Err(e) = self.store(data).await {
            warn!(error = %e, "failed to store clipboard event");
        }
    }

    /// Accept data forwarded by the GNOME Shell extension. The extension
    /// has already read a payload because Mutter requires it, so this
    /// method immediately re-applies the same TARGETS-first privacy
    /// policy before any storage operation.
    pub async fn handle_gnome_data(&self, data: ClipboardData) {
        let probe = ClipboardData {
            selection: data.selection,
            payloads: Vec::new(),
            offered_mimes: data.offered_mimes.clone(),
            source_app: data.source_app.clone(),
        };
        let verdict = self.privacy.evaluate(&probe);
        if !verdict.is_allowed() {
            debug!(?verdict, "GNOME bridge content rejected by privacy policy");
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

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
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

        // Retention policies.
        let evicted = self.db.enforce_limit(self.config.history.max_entries)?;
        if self.config.history.max_age_days > 0 {
            let cutoff = now - (self.config.history.max_age_days as i64 * 86_400);
            let _ = self.db.enforce_age(cutoff)?;
        }
        for id in evicted {
            self.sync
                .on_event(SyncEvent::EntryDeleted {
                    id,
                    content_hash: String::new(),
                })
                .await;
        }

        let entry = self.db.get(id)?;
        self.sync
            .on_event(SyncEvent::EntryUpserted(entry.clone()))
            .await;
        Ok(entry)
    }

    /// Load an entry's payloads back from the blob store.
    pub fn load_payloads(&self, entry_id: i64) -> Result<Vec<MimePayload>> {
        let mut out = Vec::new();
        for (mime, blob_ref) in self.db.blobs_of(entry_id)? {
            let data = self.blobs.get(&blob_ref)?;
            out.push(MimePayload { mime, data });
        }
        Ok(out)
    }

    /// Put a history entry back on the clipboard.
    pub async fn recall(&self, entry_id: i64) -> Result<()> {
        let entry = self.db.get(entry_id)?;
        let payloads = self.load_payloads(entry_id)?;
        let data = ClipboardData {
            selection: entry.selection,
            offered_mimes: payloads.iter().map(|p| p.mime.clone()).collect(),
            source_app: Some("panora".into()),
            payloads,
        };
        self.backend.offer(entry.selection, data).await
    }

    /// Query history for IPC clients.
    pub fn query(&self, filter: &QueryFilter) -> Result<Vec<Entry>> {
        self.db.query(filter)
    }

    /// Toggle pin state and notify sync.
    pub async fn set_pinned(&self, id: i64, pinned: bool) -> Result<()> {
        self.db.set_pinned(id, pinned)?;
        self.sync
            .on_event(SyncEvent::PinChanged { id, pinned })
            .await;
        Ok(())
    }

    /// Delete an entry (tombstone + blob cleanup) and notify sync.
    pub async fn delete(&self, id: i64) -> Result<()> {
        let entry = self.db.get(id)?;
        let blob_refs = self.db.tombstone(id)?;
        for r in blob_refs {
            self.blobs.remove(&r)?;
        }
        self.sync
            .on_event(SyncEvent::EntryDeleted {
                id,
                content_hash: entry.content_hash,
            })
            .await;
        Ok(())
    }

    /// Clear all history.
    pub async fn clear(&self) -> Result<usize> {
        let ids = self.db.clear_all()?;
        let n = ids.len();
        for id in ids {
            if let Ok(blobs) = self.db.blobs_of(id) {
                for (_, r) in blobs {
                    let _ = self.blobs.remove(&r);
                }
            }
        }
        Ok(n)
    }
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
        panora_core::model::ContentKind::Image => "[image]".to_string(),
        panora_core::model::ContentKind::FileList => {
            let n = data
                .payload_for("text/uri-list")
                .map(|p| p.data.iter().filter(|&&b| b == b'\n').count() + 1)
                .unwrap_or(0);
            format!("[{n} file(s)]")
        }
        panora_core::model::ContentKind::RichText => "[rich text]".to_string(),
        panora_core::model::ContentKind::Link => "[link]".to_string(),
        panora_core::model::ContentKind::Color => "[color]".to_string(),
        other => format!("[{}]", other.as_str()),
    }
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

    fn text_event(_text: &str) -> ClipboardEvent {
        ClipboardEvent {
            selection: Selection::Clipboard,
            offered_mimes: vec!["text/plain".into()],
            source_app: Some("test".into()),
        }
    }

    async fn offer_text(backend: &MockBackend, text: &str) {
        backend
            .offer(
                Selection::Clipboard,
                ClipboardData {
                    selection: Selection::Clipboard,
                    payloads: vec![MimePayload {
                        mime: "text/plain".into(),
                        data: text.as_bytes().to_vec(),
                    }],
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
        daemon.handle_event(text_event("merhaba panora")).await;

        let entries = daemon.query(&QueryFilter::recent(10)).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].preview, "merhaba panora");

        // Recall puts it back on the (mock) clipboard.
        daemon.recall(entries[0].id).await.unwrap();
        let back = backend
            .read(Selection::Clipboard, "text/plain")
            .await
            .unwrap();
        assert_eq!(back, b"merhaba panora");
    }

    #[tokio::test]
    async fn secret_flag_content_never_stored() {
        let dir = tempfile::tempdir().unwrap();
        let (daemon, backend) = test_daemon(&dir);
        offer_text(&backend, "super-secret-password").await;
        daemon
            .handle_event(ClipboardEvent {
                selection: Selection::Clipboard,
                offered_mimes: vec!["x-kde-passwordManagerHint".into(), "text/plain".into()],
                source_app: Some("keepassxc".into()),
            })
            .await;
        assert_eq!(daemon.query(&QueryFilter::recent(10)).unwrap().len(), 0);
    }

    #[tokio::test]
    async fn excluded_app_content_never_stored() {
        let dir = tempfile::tempdir().unwrap();
        let (daemon, backend) = test_daemon(&dir);
        offer_text(&backend, "bitwarden-secret").await;
        daemon
            .handle_event(ClipboardEvent {
                selection: Selection::Clipboard,
                offered_mimes: vec!["text/plain".into()],
                source_app: Some("Bitwarden".into()),
            })
            .await;
        assert_eq!(daemon.query(&QueryFilter::recent(10)).unwrap().len(), 0);
    }

    #[tokio::test]
    async fn private_mode_pauses_recording() {
        let dir = tempfile::tempdir().unwrap();
        let (daemon, backend) = test_daemon(&dir);
        daemon.privacy().set_private_mode(true);
        offer_text(&backend, "not recorded").await;
        daemon.handle_event(text_event("not recorded")).await;
        assert_eq!(daemon.query(&QueryFilter::recent(10)).unwrap().len(), 0);
        daemon.privacy().set_private_mode(false);
        offer_text(&backend, "recorded").await;
        daemon.handle_event(text_event("recorded")).await;
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
                    payloads: vec![MimePayload {
                        mime: "text/plain".into(),
                        data: big,
                    }],
                    offered_mimes: vec!["text/plain".into()],
                    source_app: Some("test".into()),
                },
            )
            .await
            .unwrap();
        daemon.handle_event(text_event("big")).await;
        assert_eq!(daemon.query(&QueryFilter::recent(10)).unwrap().len(), 0);
    }

    #[tokio::test]
    async fn pin_and_delete_flow() {
        let dir = tempfile::tempdir().unwrap();
        let (daemon, backend) = test_daemon(&dir);
        offer_text(&backend, "pin me").await;
        daemon.handle_event(text_event("pin me")).await;
        let id = daemon.query(&QueryFilter::recent(10)).unwrap()[0].id;

        daemon.set_pinned(id, true).await.unwrap();
        assert!(daemon.db().get(id).unwrap().pinned);

        daemon.delete(id).await.unwrap();
        assert_eq!(daemon.query(&QueryFilter::recent(10)).unwrap().len(), 0);
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
                    payloads: vec![MimePayload {
                        mime: "image/png".into(),
                        data: png.clone(),
                    }],
                    offered_mimes: vec!["image/png".into()],
                    source_app: Some("test".into()),
                },
            )
            .await
            .unwrap();
        daemon
            .handle_event(ClipboardEvent {
                selection: Selection::Clipboard,
                offered_mimes: vec!["image/png".into()],
                source_app: Some("test".into()),
            })
            .await;
        let entries = daemon.query(&QueryFilter::recent(10)).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].kind, panora_core::model::ContentKind::Image);
        let payloads = daemon.load_payloads(entries[0].id).unwrap();
        assert_eq!(payloads[0].data, png);
    }
}
