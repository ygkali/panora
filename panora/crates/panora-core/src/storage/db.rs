// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! SQLite database: entry metadata, MIME references, FTS5 search index.
//!
//! The schema carries sync-ready columns (device_id, lamport, deleted)
//! from day one per ADR 0002, so the future sync module needs no
//! migration. Preview text is stored encrypted (seal_text); the FTS
//! index is built over *decrypted* previews held in a separate table
//! that lives only in the same encrypted-at-rest database file.
//!
//! NOTE on FTS privacy: FTS5 needs plaintext to index. We therefore
//! keep the FTS table in the same database file and document that the
//! database file itself must be protected (0600 permissions). Payloads
//! (the sensitive bulk) are always encrypted in the blob store; the
//! preview column in `entries` is additionally encrypted at rest.

use super::crypto::Cipher;
use crate::error::{Error, Result};
use crate::model::{ContentKind, Entry, Selection};
use rusqlite::{params, Connection};
use std::path::Path;

/// Schema version for future migrations.
const SCHEMA_VERSION: i64 = 1;

/// Filter for history queries.
#[derive(Debug, Clone, Default)]
pub struct QueryFilter {
    /// Full-text search string (FTS5 MATCH). None = no text filter.
    pub search: Option<String>,
    /// Restrict to a content kind.
    pub kind: Option<ContentKind>,
    /// Only pinned entries.
    pub pinned_only: bool,
    /// Maximum results.
    pub limit: usize,
    /// Offset for pagination.
    pub offset: usize,
}

impl QueryFilter {
    /// A plain "recent entries" query.
    pub fn recent(limit: usize) -> Self {
        Self {
            limit,
            ..Default::default()
        }
    }
}

impl Default for &QueryFilter {
    fn default() -> Self {
        static DEFAULT: QueryFilter = QueryFilter {
            search: None,
            kind: None,
            pinned_only: false,
            limit: 50,
            offset: 0,
        };
        &DEFAULT
    }
}

/// The history database.
pub struct Database {
    conn: Connection,
    cipher: Cipher,
}

impl Database {
    /// Open or create the database at `path`. Sets restrictive file
    /// permissions (0600) since the FTS index contains plaintext
    /// previews.
    pub fn open(path: impl AsRef<Path>, cipher: Cipher) -> Result<Self> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
            Self::restrict_directory_permissions(parent)?;
        }
        let conn = Connection::open(path)?;
        let db = Self { conn, cipher };
        db.init()?;
        db.restrict_permissions(path)?;
        Ok(db)
    }

    /// Open an in-memory database (tests).
    pub fn open_in_memory(cipher: Cipher) -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        let db = Self { conn, cipher };
        db.init()?;
        Ok(db)
    }

    #[cfg(unix)]
    fn restrict_permissions(&self, path: &Path) -> Result<()> {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        Ok(())
    }

    #[cfg(not(unix))]
    fn restrict_permissions(&self, _path: &Path) -> Result<()> {
        Ok(())
    }

    fn init(&self) -> Result<()> {
        self.conn.execute_batch(
            "
            PRAGMA journal_mode = WAL;
            PRAGMA foreign_keys = ON;
            PRAGMA synchronous = NORMAL;

            CREATE TABLE IF NOT EXISTS meta (
                key   TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS entries (
                id            INTEGER PRIMARY KEY,
                content_hash  TEXT NOT NULL,
                preview       TEXT NOT NULL,          -- encrypted (seal_text hex)
                kind          TEXT NOT NULL,
                primary_mime  TEXT NOT NULL,
                size_bytes    INTEGER NOT NULL,
                source_app    TEXT,
                selection     TEXT NOT NULL DEFAULT 'clipboard',
                created_at    INTEGER NOT NULL,
                last_seen_at  INTEGER NOT NULL,
                pinned        INTEGER NOT NULL DEFAULT 0,
                -- sync-ready columns (ADR 0002) --
                device_id     TEXT NOT NULL DEFAULT '',
                lamport       INTEGER NOT NULL DEFAULT 0,
                deleted       INTEGER NOT NULL DEFAULT 0,
                UNIQUE(content_hash, selection)
            );

            CREATE INDEX IF NOT EXISTS idx_entries_last_seen
                ON entries(last_seen_at DESC);
            CREATE INDEX IF NOT EXISTS idx_entries_pinned
                ON entries(pinned, last_seen_at DESC);

            CREATE TABLE IF NOT EXISTS entry_blobs (
                entry_id INTEGER NOT NULL REFERENCES entries(id) ON DELETE CASCADE,
                mime     TEXT NOT NULL,
                blob_ref TEXT NOT NULL,
                PRIMARY KEY (entry_id, mime)
            );

            -- FTS index over decrypted previews. Kept in sync by triggers.
            CREATE VIRTUAL TABLE IF NOT EXISTS entries_fts USING fts5(
                preview,
                tokenize='unicode61'
            );
            ",
        )?;
        self.conn.execute(
            "INSERT OR REPLACE INTO meta(key, value) VALUES('schema_version', ?1)",
            params![SCHEMA_VERSION.to_string()],
        )?;
        Ok(())
    }

    /// Insert a new entry or bump `last_seen_at` when the same content
    /// is copied again (dedup). Returns the entry id.
    #[allow(clippy::too_many_arguments)]
    pub fn upsert_entry(
        &self,
        content_hash: &str,
        preview: &str,
        kind: ContentKind,
        primary_mime: &str,
        size_bytes: i64,
        source_app: Option<&str>,
        selection: Selection,
        now: i64,
        device_id: &str,
        lamport: i64,
    ) -> Result<i64> {
        let preview_aad = Self::preview_aad(content_hash);
        let sealed_preview = self
            .cipher
            .seal_text_with_aad(preview_aad.as_bytes(), preview)?;
        let tx = self.conn.unchecked_transaction()?;
        let existing: Option<i64> = tx
            .query_row(
                "SELECT id FROM entries WHERE content_hash = ?1 AND selection = ?2",
                params![content_hash, selection.as_str()],
                |row| row.get(0),
            )
            .ok();
        let id = if let Some(id) = existing {
            tx.execute(
                "UPDATE entries SET last_seen_at = ?1, deleted = 0 WHERE id = ?2",
                params![now, id],
            )?;
            id
        } else {
            tx.execute(
                "INSERT INTO entries(
                    content_hash, preview, kind, primary_mime, size_bytes,
                    source_app, selection, created_at, last_seen_at,
                    pinned, device_id, lamport, deleted
                ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,0,?10,?11,0)",
                params![
                    content_hash,
                    sealed_preview,
                    kind.as_str(),
                    primary_mime,
                    size_bytes,
                    source_app,
                    selection.as_str(),
                    now,
                    now,
                    device_id,
                    lamport,
                ],
            )?;
            let id = tx.last_insert_rowid();
            // Keep the FTS index in sync (plaintext preview, in-DB only).
            tx.execute(
                "INSERT INTO entries_fts(rowid, preview) VALUES (?1, ?2)",
                params![id, preview],
            )?;
            id
        };
        tx.commit()?;
        Ok(id)
    }

    /// Attach a blob reference to an entry.
    pub fn attach_blob(&self, entry_id: i64, mime: &str, blob_ref: &str) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO entry_blobs(entry_id, mime, blob_ref) VALUES (?1,?2,?3)",
            params![entry_id, mime, blob_ref],
        )?;
        Ok(())
    }

    /// List blob references of an entry: (mime, blob_ref) pairs.
    pub fn blobs_of(&self, entry_id: i64) -> Result<Vec<(String, String)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT mime, blob_ref FROM entry_blobs WHERE entry_id = ?1")?;
        let rows = stmt
            .query_map(params![entry_id], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Query entries. Pinned entries always sort first, then by
    /// `last_seen_at` descending. Tombstoned entries are hidden.
    pub fn query(&self, filter: &QueryFilter) -> Result<Vec<Entry>> {
        let mut sql = String::from(
            "SELECT e.id, e.content_hash, e.preview, e.kind, e.primary_mime,
                    e.size_bytes, e.source_app, e.selection, e.created_at,
                    e.last_seen_at, e.pinned, e.device_id, e.lamport, e.deleted
             FROM entries e",
        );
        let mut conditions = vec!["e.deleted = 0".to_string()];
        let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

        if let Some(search) = &filter.search {
            let trimmed = search.trim();
            if !trimmed.is_empty() {
                sql.push_str(" JOIN entries_fts ON entries_fts.rowid = e.id");
                conditions.push("entries_fts MATCH ?".to_string());
                // Quote the query to make it a safe FTS phrase search.
                params_vec.push(Box::new(format!("\"{}\"", trimmed.replace('"', ""))));
            }
        }
        if let Some(kind) = filter.kind {
            conditions.push("e.kind = ?".to_string());
            params_vec.push(Box::new(kind.as_str().to_string()));
        }
        if filter.pinned_only {
            conditions.push("e.pinned = 1".to_string());
        }

        sql.push_str(&format!(" WHERE {}", conditions.join(" AND ")));
        sql.push_str(" ORDER BY e.pinned DESC, e.last_seen_at DESC LIMIT ? OFFSET ?");
        params_vec.push(Box::new(filter.limit as i64));
        params_vec.push(Box::new(filter.offset as i64));

        let mut stmt = self.conn.prepare(&sql)?;
        let param_refs: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(|p| p.as_ref()).collect();
        let rows = stmt
            .query_map(param_refs.as_slice(), |row| self.row_to_entry(row))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Fetch a single entry by id.
    pub fn get(&self, id: i64) -> Result<Entry> {
        self.conn
            .query_row(
                "SELECT id, content_hash, preview, kind, primary_mime, size_bytes,
                        source_app, selection, created_at, last_seen_at, pinned,
                        device_id, lamport, deleted
                 FROM entries WHERE id = ?1",
                params![id],
                |row| self.row_to_entry(row),
            )
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Error::NotFound(id),
                other => Error::Database(other),
            })
    }

    fn restrict_directory_permissions(path: &Path) -> Result<()> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
        }
        #[cfg(not(unix))]
        let _ = path;
        Ok(())
    }

    fn preview_aad(content_hash: &str) -> String {
        format!("panora/entry-preview/v1/{content_hash}")
    }

    fn row_to_entry(&self, row: &rusqlite::Row) -> rusqlite::Result<Entry> {
        let content_hash: String = row.get(1)?;
        let sealed_preview: String = row.get(2)?;
        let preview = self
            .cipher
            .open_text_with_aad(Self::preview_aad(&content_hash).as_bytes(), &sealed_preview)
            .or_else(|_| self.cipher.open_text(&sealed_preview))
            .unwrap_or_else(|_| "[decryption failed]".to_string());
        let selection_str: String = row.get(7)?;
        Ok(Entry {
            id: row.get(0)?,
            content_hash,
            preview,
            kind: ContentKind::parse(&row.get::<_, String>(3)?),
            primary_mime: row.get(4)?,
            size_bytes: row.get(5)?,
            source_app: row.get(6)?,
            selection: if selection_str == "primary" {
                Selection::Primary
            } else {
                Selection::Clipboard
            },
            created_at: row.get(8)?,
            last_seen_at: row.get(9)?,
            pinned: row.get::<_, i64>(10)? != 0,
            device_id: row.get(11)?,
            lamport: row.get(12)?,
            deleted: row.get::<_, i64>(13)? != 0,
        })
    }

    /// Set or clear the pinned flag.
    pub fn set_pinned(&self, id: i64, pinned: bool) -> Result<()> {
        let changed = self.conn.execute(
            "UPDATE entries SET pinned = ?1 WHERE id = ?2",
            params![pinned as i64, id],
        )?;
        if changed == 0 {
            return Err(Error::NotFound(id));
        }
        Ok(())
    }

    /// Mark an entry deleted (tombstone). Blob cleanup is the caller's
    /// job (it owns the blob store). Returns blob refs to delete.
    pub fn tombstone(&self, id: i64) -> Result<Vec<String>> {
        let blobs: Vec<String> = self.blobs_of(id)?.into_iter().map(|(_, r)| r).collect();
        let changed = self
            .conn
            .execute("UPDATE entries SET deleted = 1 WHERE id = ?1", params![id])?;
        if changed == 0 {
            return Err(Error::NotFound(id));
        }
        self.conn
            .execute("DELETE FROM entries_fts WHERE rowid = ?1", params![id])?;
        Ok(blobs)
    }

    /// Permanently purge tombstoned entries older than `before` (unix ts).
    /// Returns blob refs that should be removed from the blob store.
    pub fn purge_tombstones(&self, before: i64) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id FROM entries WHERE deleted = 1 AND last_seen_at < ?1")?;
        let ids: Vec<i64> = stmt
            .query_map(params![before], |row| row.get(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let mut blob_refs = Vec::new();
        for id in ids {
            blob_refs.extend(self.blobs_of(id)?.into_iter().map(|(_, r)| r));
            self.conn
                .execute("DELETE FROM entry_blobs WHERE entry_id = ?1", params![id])?;
            self.conn
                .execute("DELETE FROM entries WHERE id = ?1", params![id])?;
        }
        Ok(blob_refs)
    }

    /// Evict oldest unpinned entries beyond `max_entries`. Returns the
    /// ids and blob refs of evicted entries for blob cleanup.
    pub fn enforce_limit(&self, max_entries: usize) -> Result<Vec<i64>> {
        let mut stmt = self.conn.prepare(
            "SELECT id FROM entries
             WHERE deleted = 0 AND pinned = 0
             ORDER BY last_seen_at DESC
             LIMIT -1 OFFSET ?1",
        )?;
        let ids: Vec<i64> = stmt
            .query_map(params![max_entries as i64], |row| row.get(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        for &id in &ids {
            self.tombstone(id)?;
        }
        Ok(ids)
    }

    /// Evict unpinned entries older than `before` (unix ts).
    pub fn enforce_age(&self, before: i64) -> Result<Vec<i64>> {
        let mut stmt = self.conn.prepare(
            "SELECT id FROM entries WHERE deleted = 0 AND pinned = 0 AND last_seen_at < ?1",
        )?;
        let ids: Vec<i64> = stmt
            .query_map(params![before], |row| row.get(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        for &id in &ids {
            self.tombstone(id)?;
        }
        Ok(ids)
    }

    /// Clear the history, keeping pinned entries. Returns the cleared ids.
    ///
    /// Pinning means "keep this", which is why `enforce_limit` and
    /// `enforce_age` skip pinned rows. A manual clear honours the same
    /// contract, so a pinned entry survives it; unpin first to remove one.
    pub fn clear_all(&self) -> Result<Vec<i64>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id FROM entries WHERE deleted = 0 AND pinned = 0")?;
        let ids: Vec<i64> = stmt
            .query_map([], |row| row.get(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        for &id in &ids {
            self.tombstone(id)?;
        }
        Ok(ids)
    }

    /// Total number of visible entries.
    pub fn count(&self) -> Result<i64> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM entries WHERE deleted = 0",
            [],
            |row| row.get(0),
        )?;
        Ok(n)
    }

    /// Highest lamport clock seen (sync-ready; v1.0 keeps it monotonic).
    pub fn max_lamport(&self) -> Result<i64> {
        let n: i64 =
            self.conn
                .query_row("SELECT COALESCE(MAX(lamport), 0) FROM entries", [], |row| {
                    row.get(0)
                })?;
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::crypto::MasterKey;

    fn db() -> Database {
        Database::open_in_memory(Cipher::new(&MasterKey::generate())).unwrap()
    }

    fn insert(db: &Database, preview: &str, kind: ContentKind, ts: i64) -> i64 {
        let hash = crate::storage::crypto::content_hash(preview.as_bytes());
        db.upsert_entry(
            &hash,
            preview,
            kind,
            "text/plain",
            preview.len() as i64,
            Some("test-app"),
            Selection::Clipboard,
            ts,
            "dev0",
            1,
        )
        .unwrap()
    }

    #[test]
    fn insert_and_get() {
        let db = db();
        let id = insert(&db, "hello panora", ContentKind::Text, 1000);
        let e = db.get(id).unwrap();
        assert_eq!(e.preview, "hello panora");
        assert_eq!(e.kind, ContentKind::Text);
        assert!(!e.pinned);
        assert_eq!(e.device_id, "dev0");
        assert_eq!(e.lamport, 1);
        assert!(!e.deleted);
    }

    #[test]
    fn dedup_bumps_last_seen() {
        let db = db();
        let id1 = insert(&db, "same text", ContentKind::Text, 1000);
        let id2 = insert(&db, "same text", ContentKind::Text, 2000);
        assert_eq!(id1, id2, "same content must dedup to one row");
        assert_eq!(db.count().unwrap(), 1);
        assert_eq!(db.get(id1).unwrap().last_seen_at, 2000);
    }

    #[test]
    fn pinned_sorts_first() {
        let db = db();
        insert(&db, "old pinned", ContentKind::Text, 100);
        let recent = insert(&db, "recent unpinned", ContentKind::Text, 9999);
        let pinned_id = db.query(&QueryFilter::recent(10)).unwrap()[1].id;
        db.set_pinned(pinned_id, true).unwrap();
        let entries = db.query(&QueryFilter::recent(10)).unwrap();
        assert_eq!(entries[0].id, pinned_id);
        assert_eq!(entries[1].id, recent);
    }

    #[test]
    fn fts_search_finds_text() {
        let db = db();
        insert(&db, "the quick brown fox", ContentKind::Text, 1);
        insert(&db, "lazy dog sleeps", ContentKind::Text, 2);
        insert(&db, "turkish merhaba dunya", ContentKind::Text, 3);
        let hits = db
            .query(&QueryFilter {
                search: Some("merhaba".into()),
                ..QueryFilter::recent(10)
            })
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].preview, "turkish merhaba dunya");
    }

    #[test]
    fn search_with_special_chars_does_not_crash() {
        let db = db();
        insert(&db, "some content", ContentKind::Text, 1);
        for q in ["\"quoted\"", "OR AND NOT", "col* wildcard", "'single'"] {
            let _ = db
                .query(&QueryFilter {
                    search: Some(q.into()),
                    ..QueryFilter::recent(10)
                })
                .unwrap();
        }
    }

    #[test]
    fn tombstone_hides_entry() {
        let db = db();
        let id = insert(&db, "to delete", ContentKind::Text, 1);
        db.attach_blob(id, "text/plain", "blobref1").unwrap();
        let blobs = db.tombstone(id).unwrap();
        assert_eq!(blobs, vec!["blobref1"]);
        assert_eq!(db.count().unwrap(), 0);
        assert!(db.query(&QueryFilter::recent(10)).unwrap().is_empty());
        // FTS row removed too:
        let hits = db
            .query(&QueryFilter {
                search: Some("delete".into()),
                ..QueryFilter::recent(10)
            })
            .unwrap();
        assert!(hits.is_empty());
    }

    #[test]
    fn purge_tombstones_removes_rows() {
        let db = db();
        let id = insert(&db, "purge me", ContentKind::Text, 100);
        db.attach_blob(id, "text/plain", "ref-x").unwrap();
        db.tombstone(id).unwrap();
        let refs = db.purge_tombstones(200).unwrap();
        assert_eq!(refs, vec!["ref-x"]);
        assert!(db.get(id).is_err());
    }

    #[test]
    fn enforce_limit_keeps_pinned() {
        let db = db();
        for i in 0..10 {
            insert(&db, &format!("entry {i}"), ContentKind::Text, i as i64);
        }
        let first = db.query(&QueryFilter::recent(100)).unwrap()[9].id; // oldest
        db.set_pinned(first, true).unwrap();
        // 10 entries, 1 pinned -> 9 unpinned; keeping 5 unpinned evicts 4.
        let evicted = db.enforce_limit(5).unwrap();
        assert_eq!(evicted.len(), 4);
        assert_eq!(db.count().unwrap(), 6);
        assert!(db.get(first).is_ok(), "pinned entry must survive eviction");
    }

    #[test]
    fn enforce_age() {
        let db = db();
        insert(&db, "ancient", ContentKind::Text, 10);
        insert(&db, "fresh", ContentKind::Text, 9000);
        let evicted = db.enforce_age(5000).unwrap();
        assert_eq!(evicted.len(), 1);
        assert_eq!(db.count().unwrap(), 1);
    }

    #[test]
    fn clear_all_empties_history() {
        let db = db();
        insert(&db, "one", ContentKind::Text, 1);
        insert(&db, "two", ContentKind::Text, 2);
        let ids = db.clear_all().unwrap();
        assert_eq!(db.count().unwrap(), 0);
        assert_eq!(ids.len(), 2);
    }

    #[test]
    fn clear_all_keeps_pinned() {
        let db = db();
        let keep = insert(&db, "pinned", ContentKind::Text, 1);
        insert(&db, "transient", ContentKind::Text, 2);
        db.set_pinned(keep, true).unwrap();

        let ids = db.clear_all().unwrap();

        assert_eq!(ids.len(), 1, "only the unpinned entry is cleared");
        assert!(!ids.contains(&keep));
        assert_eq!(db.count().unwrap(), 1, "the pinned entry survives");
    }

    #[test]
    fn kind_filter_works() {
        let db = db();
        insert(&db, "plain", ContentKind::Text, 1);
        insert(&db, "#ff0000", ContentKind::Color, 2);
        let colors = db
            .query(&QueryFilter {
                kind: Some(ContentKind::Color),
                ..QueryFilter::recent(10)
            })
            .unwrap();
        assert_eq!(colors.len(), 1);
        assert_eq!(colors[0].kind, ContentKind::Color);
    }

    #[test]
    fn max_lamport_tracks() {
        let db = db();
        assert_eq!(db.max_lamport().unwrap(), 0);
        let hash_a = crate::storage::crypto::content_hash(b"a");
        db.upsert_entry(
            &hash_a,
            "a",
            ContentKind::Text,
            "text/plain",
            1,
            None,
            Selection::Clipboard,
            1,
            "dev0",
            41,
        )
        .unwrap();
        let hash_b = crate::storage::crypto::content_hash(b"b");
        db.upsert_entry(
            &hash_b,
            "b",
            ContentKind::Text,
            "text/plain",
            1,
            None,
            Selection::Clipboard,
            2,
            "dev0",
            42,
        )
        .unwrap();
        assert_eq!(db.max_lamport().unwrap(), 42);
    }

    #[test]
    fn preview_is_encrypted_at_rest() {
        let db = db();
        insert(&db, "sensitive preview text", ContentKind::Text, 1);
        // Read the raw column directly: it must NOT contain plaintext.
        let raw: String = db
            .conn
            .query_row("SELECT preview FROM entries LIMIT 1", [], |r| r.get(0))
            .unwrap();
        assert!(!raw.contains("sensitive"));
        assert!(raw.len() > 48, "sealed hex should be long: {raw}");
    }
}
