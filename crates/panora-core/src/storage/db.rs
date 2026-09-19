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

/// Schema version written to `meta`. Bump it together with `MIGRATIONS`.
pub const SCHEMA_VERSION: i64 = 4;

/// One schema step: SQL run inside a transaction, or Rust for what SQL alone
/// cannot do (rebuilding the FTS table from the encrypted previews).
enum Migration {
    Sql(&'static str),
    Code(fn(&Database) -> Result<()>),
}

/// Ordered schema migrations for databases created by older releases, as
/// `(version_after, step)`. `init` always creates the current schema, so
/// these only run on an existing file whose stored version is lower; each
/// one runs in its own transaction and stamps its version with it. The last
/// entry's version must equal `SCHEMA_VERSION`.
const MIGRATIONS: &[(i64, Migration)] = &[
    // 1.3.0: deletions keep their time so an undo window can be measured
    // from the deletion, not from when the entry was last copied.
    (
        2,
        Migration::Sql("ALTER TABLE entries ADD COLUMN deleted_at INTEGER NOT NULL DEFAULT 0"),
    ),
    // 1.3.0: the search index gains a `content` column (text beyond the
    // 500-character preview) and a tokenizer that folds diacritics, which
    // means recreating the FTS table and re-adding every live preview.
    (3, Migration::Code(rebuild_search_index)),
    // 1.3.0: entries that look like secrets are flagged so they can be
    // masked and expired (`crate::sensitive`).
    (
        4,
        Migration::Sql("ALTER TABLE entries ADD COLUMN sensitive INTEGER NOT NULL DEFAULT 0"),
    ),
];

/// FTS5 table definition shared by `init` and the version 3 migration.
const FTS_TABLE_SQL: &str = "CREATE VIRTUAL TABLE IF NOT EXISTS entries_fts USING fts5(
    preview,
    content,
    tokenize='unicode61 remove_diacritics 2'
)";

/// Recreate the search index with the current definition and refill it
/// from the live previews. Content beyond the preview is not available here
/// (it lives in the encrypted blobs), so older entries stay searchable by
/// their preview only until they are copied again.
fn rebuild_search_index(db: &Database) -> Result<()> {
    db.conn.execute_batch("DROP TABLE IF EXISTS entries_fts")?;
    db.conn.execute_batch(FTS_TABLE_SQL)?;
    let mut stmt = db
        .conn
        .prepare("SELECT id, content_hash, preview FROM entries WHERE deleted = 0")?;
    let rows: Vec<(i64, String, String)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    for (id, hash, sealed) in rows {
        let preview = db
            .cipher
            .open_text_with_aad(Database::preview_aad(&hash).as_bytes(), &sealed)
            .or_else(|_| db.cipher.open_text(&sealed))?;
        db.conn.execute(
            "INSERT INTO entries_fts(rowid, preview) VALUES (?1, ?2)",
            params![id, preview],
        )?;
    }
    Ok(())
}

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

impl std::fmt::Debug for Database {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Database")
            .field("key_fingerprint", &self.cipher.fingerprint())
            .finish_non_exhaustive()
    }
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
        db.check_integrity()?;
        db.prepare(Some(path), MIGRATIONS, SCHEMA_VERSION)?;
        db.restrict_permissions(path)?;
        Ok(db)
    }

    /// Open an in-memory database (tests).
    pub fn open_in_memory(cipher: Cipher) -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        let db = Self { conn, cipher };
        db.prepare(None, MIGRATIONS, SCHEMA_VERSION)?;
        Ok(db)
    }

    /// Refuse a damaged file before touching it. `quick_check` is the
    /// integrity check minus the index cross-checks, cheap enough for every
    /// start; a file that is not SQLite at all fails here too.
    fn check_integrity(&self) -> Result<()> {
        let verdict: String = self
            .conn
            .query_row("PRAGMA quick_check", [], |row| row.get(0))?;
        if verdict != "ok" {
            return Err(Error::Storage(format!(
                "the history database failed its integrity check: {verdict}"
            )));
        }
        Ok(())
    }

    /// Bring the file to the current schema and bind it to the cipher's key.
    ///
    /// A fresh file gets the current schema and is stamped with `target`. An
    /// existing file is checked against the key first (so a foreign database
    /// is never migrated or backed up), refused when its schema is newer than
    /// this build, and otherwise migrated step by step after a `VACUUM INTO`
    /// backup next to it. `migrations` and `target` are parameters only so
    /// the tests can exercise the path with a fake migration.
    fn prepare(
        &self,
        path: Option<&Path>,
        migrations: &[(i64, Migration)],
        target: i64,
    ) -> Result<()> {
        let stored = self.stored_schema_version()?;
        if stored == 0 {
            self.init()?;
            self.set_schema_version(target)?;
            self.bind_key()?;
            return Ok(());
        }
        if stored > target {
            return Err(Error::Storage(format!(
                "the history database uses schema version {stored}, newer than this \
                 version of Panora supports ({target}); upgrade Panora"
            )));
        }
        self.check_key()?;
        if stored < target {
            if let Some(path) = path {
                self.backup_before_migration(path, stored)?;
            }
            for (version, step) in migrations.iter().filter(|(v, _)| *v > stored) {
                let tx = self.conn.unchecked_transaction()?;
                match step {
                    Migration::Sql(sql) => tx.execute_batch(sql)?,
                    // Runs on the same connection, hence inside `tx`.
                    Migration::Code(run) => run(self)?,
                }
                tx.execute(
                    "INSERT OR REPLACE INTO meta(key, value) VALUES('schema_version', ?1)",
                    params![version.to_string()],
                )?;
                tx.commit()?;
            }
            if self.stored_schema_version()? != target {
                return Err(Error::Storage(format!(
                    "no migration path from schema version {stored} to {target}"
                )));
            }
        }
        // Idempotent: objects an older file never had (all CREATE IF NOT
        // EXISTS) come into being here without a migration entry.
        self.init()?;
        Ok(())
    }

    /// Schema version recorded in the file, or 0 when nothing is there yet.
    pub fn schema_version(&self) -> Result<i64> {
        self.stored_schema_version()
    }

    fn stored_schema_version(&self) -> Result<i64> {
        let has_meta: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'meta'",
            [],
            |row| row.get(0),
        )?;
        if has_meta == 0 {
            return Ok(0);
        }
        Ok(self
            .meta("schema_version")?
            .and_then(|value| value.parse().ok())
            .unwrap_or(0))
    }

    fn set_schema_version(&self, version: i64) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO meta(key, value) VALUES('schema_version', ?1)",
            params![version.to_string()],
        )?;
        Ok(())
    }

    fn meta(&self, key: &str) -> Result<Option<String>> {
        use rusqlite::OptionalExtension as _;
        Ok(self
            .conn
            .query_row(
                "SELECT value FROM meta WHERE key = ?1",
                params![key],
                |row| row.get(0),
            )
            .optional()?)
    }

    /// Record which key encrypts this file (first open only).
    fn bind_key(&self) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO meta(key, value) VALUES('key_fingerprint', ?1)",
            params![self.cipher.fingerprint()],
        )?;
        Ok(())
    }

    /// Fail closed when the file was encrypted with another key. Without a
    /// stored fingerprint (files written before it existed) the newest
    /// preview is decrypted as the proof instead, then the key is bound.
    fn check_key(&self) -> Result<()> {
        let mismatch = || {
            Error::Storage(
                "the history was encrypted with a different master key (was the keyring \
                 reset?); restore the keyring, or move the Panora data directory away to \
                 start a new history"
                    .into(),
            )
        };
        match self.meta("key_fingerprint")? {
            Some(stored) if stored == self.cipher.fingerprint() => Ok(()),
            Some(_) => Err(mismatch()),
            None => {
                use rusqlite::OptionalExtension as _;
                let probe: Option<(String, String)> = self
                    .conn
                    .query_row(
                        "SELECT content_hash, preview FROM entries ORDER BY id DESC LIMIT 1",
                        [],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .optional()?;
                if let Some((hash, sealed)) = probe {
                    let opened = self
                        .cipher
                        .open_text_with_aad(Self::preview_aad(&hash).as_bytes(), &sealed)
                        .or_else(|_| self.cipher.open_text(&sealed));
                    if opened.is_err() {
                        return Err(mismatch());
                    }
                }
                self.bind_key()
            }
        }
    }

    /// Copy the file next to itself before a migration touches it. Uses
    /// `VACUUM INTO`, which produces a consistent copy even with a WAL.
    fn backup_before_migration(&self, path: &Path, stored: i64) -> Result<()> {
        let backup = path.with_extension(format!("db.bak-v{stored}"));
        if backup.exists() {
            std::fs::remove_file(&backup)?;
        }
        self.conn.execute(
            "VACUUM INTO ?1",
            params![backup.to_string_lossy().into_owned()],
        )?;
        self.restrict_permissions(&backup)?;
        Ok(())
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
                -- looks like a secret (crate::sensitive): masked, expiring
                sensitive     INTEGER NOT NULL DEFAULT 0,
                -- sync-ready columns (ADR 0002) --
                device_id     TEXT NOT NULL DEFAULT '',
                lamport       INTEGER NOT NULL DEFAULT 0,
                deleted       INTEGER NOT NULL DEFAULT 0,
                -- when the tombstone was set; drives the undo grace period
                deleted_at    INTEGER NOT NULL DEFAULT 0,
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

            ",
        )?;
        // FTS index over decrypted previews and, for text entries, the text
        // beyond the preview; every write path above keeps it in step.
        self.conn.execute_batch(FTS_TABLE_SQL)?;
        Ok(())
    }

    /// Index the text of an entry beyond its preview, so a search matches
    /// words anywhere in it. Only the FTS table changes; the encrypted
    /// preview column stays as it is.
    pub fn index_content(&self, id: i64, content: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE entries_fts SET content = ?2 WHERE rowid = ?1",
            params![id, content],
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
        let existing: Option<(i64, bool)> = tx
            .query_row(
                "SELECT id, deleted FROM entries WHERE content_hash = ?1 AND selection = ?2",
                params![content_hash, selection.as_str()],
                |row| Ok((row.get(0)?, row.get::<_, i64>(1)? != 0)),
            )
            .ok();
        let id = if let Some((id, false)) = existing {
            tx.execute(
                // Same-second re-copies must still move to the top, so the
                // timestamp never ties with the current newest row.
                "UPDATE entries SET last_seen_at = MAX(?1,
                    (SELECT COALESCE(MAX(last_seen_at), 0) FROM entries WHERE id != ?2) + 1)
                 WHERE id = ?2",
                params![now, id],
            )?;
            id
        } else if let Some((id, true)) = existing {
            // Revive a tombstoned row: its blobs were removed with it, so the
            // caller re-attaches payloads, and the FTS row must come back too.
            tx.execute(
                "UPDATE entries SET preview = ?1, kind = ?2, primary_mime = ?3,
                        size_bytes = ?4, source_app = ?5, created_at = ?6,
                        last_seen_at = ?6, pinned = 0, device_id = ?7,
                        lamport = ?8, deleted = 0, sensitive = 0
                 WHERE id = ?9",
                params![
                    sealed_preview,
                    kind.as_str(),
                    primary_mime,
                    size_bytes,
                    source_app,
                    now,
                    device_id,
                    lamport,
                    id
                ],
            )?;
            tx.execute("DELETE FROM entries_fts WHERE rowid = ?1", params![id])?;
            tx.execute(
                "INSERT INTO entries_fts(rowid, preview) VALUES (?1, ?2)",
                params![id, preview],
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

    /// Flag an entry as sensitive (see `crate::sensitive`).
    pub fn mark_sensitive(&self, id: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE entries SET sensitive = 1 WHERE id = ?1",
            params![id],
        )?;
        Ok(())
    }

    /// Evict unpinned sensitive entries last seen before `before`; no undo
    /// window, like the other retention rules. Returns the evicted ids.
    pub fn expire_sensitive(&self, before: i64) -> Result<Vec<i64>> {
        let mut stmt = self.conn.prepare(
            "SELECT id FROM entries
             WHERE deleted = 0 AND pinned = 0 AND sensitive = 1 AND last_seen_at < ?1",
        )?;
        let ids: Vec<i64> = stmt
            .query_map(params![before], |row| row.get(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        for &id in &ids {
            self.tombstone(id, 0)?;
        }
        Ok(ids)
    }

    /// Drop one blob reference of an entry (the blob itself is the caller's
    /// to remove once nothing else references it).
    pub fn detach_blob(&self, entry_id: i64, mime: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM entry_blobs WHERE entry_id = ?1 AND mime = ?2",
            params![entry_id, mime],
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
                    e.last_seen_at, e.pinned, e.device_id, e.lamport, e.deleted,
                    e.sensitive
             FROM entries e",
        );
        let mut conditions = vec!["e.deleted = 0".to_string()];
        let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

        if let Some(search) = &filter.search {
            if let Some(expression) = fts_query(search) {
                sql.push_str(" JOIN entries_fts ON entries_fts.rowid = e.id");
                conditions.push("entries_fts MATCH ?".to_string());
                params_vec.push(Box::new(expression));
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
        sql.push_str(" ORDER BY e.pinned DESC, e.last_seen_at DESC, e.id DESC LIMIT ? OFFSET ?");
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
                        device_id, lamport, deleted, sensitive
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
            sensitive: row.get::<_, i64>(14)? != 0,
            device_id: row.get(11)?,
            lamport: row.get(12)?,
            deleted: row.get::<_, i64>(13)? != 0,
        })
    }

    /// Most recently seen visible entry of a selection, if any.
    pub fn latest(&self, selection: Selection) -> Result<Option<Entry>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, content_hash, preview, kind, primary_mime, size_bytes,
                    source_app, selection, created_at, last_seen_at, pinned,
                    device_id, lamport, deleted, sensitive
             FROM entries WHERE deleted = 0 AND selection = ?1
             ORDER BY last_seen_at DESC LIMIT 1",
        )?;
        let mut rows = stmt.query_map(params![selection.as_str()], |row| self.row_to_entry(row))?;
        match rows.next() {
            Some(row) => Ok(Some(row?)),
            None => Ok(None),
        }
    }

    /// Bump `last_seen_at` (a recalled entry moves back to the top).
    pub fn touch(&self, id: i64, now: i64) -> Result<()> {
        let changed = self.conn.execute(
            "UPDATE entries SET last_seen_at = MAX(?1,
                (SELECT COALESCE(MAX(last_seen_at), 0) FROM entries WHERE id != ?2) + 1)
             WHERE id = ?2 AND deleted = 0",
            params![now, id],
        )?;
        if changed == 0 {
            return Err(Error::NotFound(id));
        }
        Ok(())
    }

    /// Number of entries (visible or tombstoned) still referencing a blob.
    /// Blobs are content-addressed and shared, so a blob may only be removed
    /// from disk once this reaches zero.
    pub fn blob_ref_count(&self, blob_ref: &str) -> Result<i64> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM entry_blobs WHERE blob_ref = ?1",
            params![blob_ref],
            |row| row.get(0),
        )?;
        Ok(n)
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

    /// Mark an entry deleted (tombstone) at time `now`. Blob cleanup is the
    /// caller's job (it owns the blob store). Returns blob refs to delete.
    pub fn tombstone(&self, id: i64, now: i64) -> Result<Vec<String>> {
        let blobs: Vec<String> = self.blobs_of(id)?.into_iter().map(|(_, r)| r).collect();
        let changed = self.conn.execute(
            "UPDATE entries SET deleted = 1, deleted_at = ?2 WHERE id = ?1",
            params![id, now],
        )?;
        if changed == 0 {
            return Err(Error::NotFound(id));
        }
        self.conn
            .execute("DELETE FROM entries_fts WHERE rowid = ?1", params![id])?;
        Ok(blobs)
    }

    /// Undo a deletion: the tombstone is lifted and the entry is searchable
    /// again. Its blobs are still on disk as long as the tombstone has not
    /// been purged, which is what the grace period in the daemon guarantees.
    pub fn restore(&self, id: i64) -> Result<()> {
        let entry = self.get(id)?;
        if !entry.deleted {
            return Ok(());
        }
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "UPDATE entries SET deleted = 0, deleted_at = 0 WHERE id = ?1",
            params![id],
        )?;
        tx.execute("DELETE FROM entries_fts WHERE rowid = ?1", params![id])?;
        tx.execute(
            "INSERT INTO entries_fts(rowid, preview) VALUES (?1, ?2)",
            params![id, entry.preview],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Permanently purge entries tombstoned before `before` (unix ts).
    /// Returns the blob refs that no longer have any reference and can be
    /// removed from the blob store.
    pub fn purge_tombstones(&self, before: i64) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id FROM entries WHERE deleted = 1 AND deleted_at < ?1")?;
        let ids: Vec<i64> = stmt
            .query_map(params![before], |row| row.get(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        self.purge(&ids)
    }

    /// Permanently remove the given tombstoned rows. Returns blob refs that
    /// became unreferenced.
    pub fn purge(&self, ids: &[i64]) -> Result<Vec<String>> {
        let tx = self.conn.unchecked_transaction()?;
        let mut candidates = Vec::new();
        for &id in ids {
            let tombstoned: bool = tx
                .query_row(
                    "SELECT deleted FROM entries WHERE id = ?1",
                    params![id],
                    |row| row.get::<_, i64>(0),
                )
                .map(|d| d != 0)
                .unwrap_or(false);
            if !tombstoned {
                continue;
            }
            let mut stmt = tx.prepare("SELECT blob_ref FROM entry_blobs WHERE entry_id = ?1")?;
            let refs: Vec<String> = stmt
                .query_map(params![id], |row| row.get(0))?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            candidates.extend(refs);
            tx.execute("DELETE FROM entry_blobs WHERE entry_id = ?1", params![id])?;
            tx.execute("DELETE FROM entries_fts WHERE rowid = ?1", params![id])?;
            tx.execute(
                "DELETE FROM entries WHERE id = ?1 AND deleted = 1",
                params![id],
            )?;
        }
        candidates.sort();
        candidates.dedup();
        let mut orphaned = Vec::new();
        for blob_ref in candidates {
            let n: i64 = tx.query_row(
                "SELECT COUNT(*) FROM entry_blobs WHERE blob_ref = ?1",
                params![blob_ref],
                |row| row.get(0),
            )?;
            if n == 0 {
                orphaned.push(blob_ref);
            }
        }
        tx.commit()?;
        Ok(orphaned)
    }

    /// Total plaintext payload bytes of visible entries (settings/status).
    pub fn total_size(&self) -> Result<i64> {
        let n: i64 = self.conn.query_row(
            "SELECT COALESCE(SUM(size_bytes), 0) FROM entries WHERE deleted = 0",
            [],
            |row| row.get(0),
        )?;
        Ok(n)
    }

    /// Evict oldest unpinned entries beyond `max_entries`. Returns the
    /// evicted ids. Retention is not a user action, so these tombstones get
    /// no undo window (`deleted_at = 0`) and go at the next purge.
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
            self.tombstone(id, 0)?;
        }
        Ok(ids)
    }

    /// Evict unpinned entries last seen before `before` (unix ts); no undo
    /// window, like `enforce_limit`.
    pub fn enforce_age(&self, before: i64) -> Result<Vec<i64>> {
        let mut stmt = self.conn.prepare(
            "SELECT id FROM entries WHERE deleted = 0 AND pinned = 0 AND last_seen_at < ?1",
        )?;
        let ids: Vec<i64> = stmt
            .query_map(params![before], |row| row.get(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        for &id in &ids {
            self.tombstone(id, 0)?;
        }
        Ok(ids)
    }

    /// Evict the oldest unpinned entries until the payload bytes of the
    /// live history fit in `max_bytes`. Pinned entries count towards the
    /// total but never go; once one entry has to go, everything older goes
    /// with it. No undo window, like the other retention rules.
    pub fn enforce_total_bytes(&self, max_bytes: u64) -> Result<Vec<i64>> {
        let pinned: i64 = self.conn.query_row(
            "SELECT COALESCE(SUM(size_bytes), 0) FROM entries WHERE deleted = 0 AND pinned = 1",
            [],
            |row| row.get(0),
        )?;
        let mut used = u64::try_from(pinned).unwrap_or(0);
        let mut stmt = self.conn.prepare(
            "SELECT id, size_bytes FROM entries
             WHERE deleted = 0 AND pinned = 0
             ORDER BY last_seen_at DESC, id DESC",
        )?;
        let rows: Vec<(i64, i64)> = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let mut evicted = Vec::new();
        for (id, size) in rows {
            let size = u64::try_from(size).unwrap_or(0);
            if !evicted.is_empty() || used.saturating_add(size) > max_bytes {
                evicted.push(id);
            } else {
                used += size;
            }
        }
        for &id in &evicted {
            self.tombstone(id, 0)?;
        }
        Ok(evicted)
    }

    /// Every blob reference any row (live or tombstoned) still holds.
    pub fn referenced_blobs(&self) -> Result<std::collections::HashSet<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT DISTINCT blob_ref FROM entry_blobs")?;
        let refs = stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<_, _>>()?;
        Ok(refs)
    }

    /// Routine upkeep: refresh the planner statistics, fold the WAL back
    /// into the main file, and reclaim free pages once more than a quarter
    /// of the file is unused (a full VACUUM rewrites the file, so not on
    /// every run).
    pub fn maintain(&self) -> Result<()> {
        self.conn.execute_batch("PRAGMA optimize")?;
        let _: (i64, i64, i64) =
            self.conn
                .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?))
                })?;
        let page_count: i64 = self
            .conn
            .query_row("PRAGMA page_count", [], |row| row.get(0))?;
        let freelist: i64 = self
            .conn
            .query_row("PRAGMA freelist_count", [], |row| row.get(0))?;
        if page_count > 0 && freelist * 4 > page_count {
            self.conn.execute_batch("VACUUM")?;
        }
        Ok(())
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
        // Clearing is confirmed in a dialog first, so it is final: no undo
        // window either.
        for &id in &ids {
            self.tombstone(id, 0)?;
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

/// Build an FTS5 MATCH expression from free text typed by the user.
///
/// Every whitespace-separated token becomes a quoted prefix term, so the
/// list narrows as the user types (`mer` finds `merhaba`) and no FTS
/// operator or punctuation in the input can change the query shape.
pub fn fts_query(search: &str) -> Option<String> {
    let terms: Vec<String> = search
        .split_whitespace()
        .map(|token| token.replace('"', ""))
        .filter(|token| !token.is_empty())
        .take(16)
        .map(|token| format!("\"{token}\"*"))
        .collect();
    if terms.is_empty() {
        None
    } else {
        Some(terms.join(" "))
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
        let blobs = db.tombstone(id, 150).unwrap();
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
        db.tombstone(id, 150).unwrap();
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
    fn prefix_search_matches_while_typing() {
        let db = db();
        insert(&db, "merhaba dünya", ContentKind::Text, 1);
        insert(&db, "unrelated", ContentKind::Text, 2);
        for q in ["mer", "merhaba dün", "DÜNYA"] {
            let hits = db
                .query(&QueryFilter {
                    search: Some(q.into()),
                    ..QueryFilter::recent(10)
                })
                .unwrap();
            assert_eq!(hits.len(), 1, "query {q:?}");
        }
        assert_eq!(fts_query("  "), None);
        assert_eq!(fts_query("a \"b\" c"), Some("\"a\"* \"b\"* \"c\"*".into()));
    }

    #[test]
    fn revived_tombstone_is_searchable_again() {
        let db = db();
        let id = insert(&db, "come back", ContentKind::Text, 1);
        db.set_pinned(id, true).unwrap();
        db.tombstone(id, 150).unwrap();
        let again = insert(&db, "come back", ContentKind::Text, 5);
        assert_eq!(again, id);
        let e = db.get(id).unwrap();
        assert!(!e.deleted);
        assert!(!e.pinned, "revived rows start unpinned");
        assert_eq!(e.created_at, 5);
        let hits = db
            .query(&QueryFilter {
                search: Some("come".into()),
                ..QueryFilter::recent(10)
            })
            .unwrap();
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn purge_reports_only_unreferenced_blobs() {
        let db = db();
        let a = insert(&db, "a", ContentKind::Text, 1);
        let b = insert(&db, "b", ContentKind::Text, 2);
        db.attach_blob(a, "text/plain", "shared").unwrap();
        db.attach_blob(a, "text/html", "only-a").unwrap();
        db.attach_blob(b, "text/plain", "shared").unwrap();
        assert_eq!(db.blob_ref_count("shared").unwrap(), 2);
        db.tombstone(a, 150).unwrap();
        let orphaned = db.purge(&[a]).unwrap();
        assert_eq!(orphaned, vec!["only-a"]);
        assert_eq!(db.blob_ref_count("shared").unwrap(), 1);
        assert!(db.get(a).is_err());
        // Visible rows are never purged.
        assert!(db.purge(&[b]).unwrap().is_empty());
        assert!(db.get(b).is_ok());
    }

    #[test]
    fn touch_moves_entry_to_top() {
        let db = db();
        assert!(db.latest(Selection::Clipboard).unwrap().is_none());
        let old = insert(&db, "old", ContentKind::Text, 1);
        insert(&db, "new", ContentKind::Text, 2);
        db.touch(old, 10).unwrap();
        assert_eq!(db.query(&QueryFilter::recent(2)).unwrap()[0].id, old);
        assert_eq!(db.latest(Selection::Clipboard).unwrap().unwrap().id, old);
        assert!(db.latest(Selection::Primary).unwrap().is_none());
        assert!(db.touch(9999, 1).is_err());
        assert_eq!(db.total_size().unwrap(), 6);
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

#[cfg(test)]
mod lifecycle_tests {
    use super::*;
    use crate::storage::crypto::MasterKey;

    fn open_with(path: &Path, key: &MasterKey) -> Result<Database> {
        Database::open(path, Cipher::new(key))
    }

    fn insert_one(db: &Database, preview: &str) {
        let hash = crate::storage::crypto::content_hash(preview.as_bytes());
        db.upsert_entry(
            &hash,
            preview,
            ContentKind::Text,
            "text/plain",
            preview.len() as i64,
            None,
            Selection::Clipboard,
            1,
            "dev0",
            1,
        )
        .unwrap();
    }

    #[test]
    fn fresh_database_is_stamped_with_version_and_key() {
        let dir = tempfile::tempdir().unwrap();
        let key = MasterKey::generate();
        let db = open_with(&dir.path().join("history.db"), &key).unwrap();
        assert_eq!(db.schema_version().unwrap(), SCHEMA_VERSION);
        assert_eq!(
            db.meta("key_fingerprint").unwrap().as_deref(),
            Some(db.cipher.fingerprint())
        );
        assert_eq!(db.cipher.fingerprint().len(), 16);
    }

    #[test]
    fn older_schema_is_migrated_after_a_backup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.db");
        let key = MasterKey::generate();
        {
            let db = open_with(&path, &key).unwrap();
            insert_one(&db, "before the migration");
        }
        let conn = Connection::open(&path).unwrap();
        let db = Database {
            conn,
            cipher: Cipher::new(&key),
        };
        let fake = [(
            SCHEMA_VERSION + 1,
            Migration::Sql("ALTER TABLE entries ADD COLUMN migrated INTEGER NOT NULL DEFAULT 7"),
        )];
        db.prepare(Some(&path), &fake, SCHEMA_VERSION + 1).unwrap();
        assert_eq!(db.schema_version().unwrap(), SCHEMA_VERSION + 1);
        let migrated: i64 = db
            .conn
            .query_row("SELECT migrated FROM entries LIMIT 1", [], |row| row.get(0))
            .unwrap();
        assert_eq!(migrated, 7);
        assert_eq!(db.count().unwrap(), 1, "data survives the migration");

        let backup = path.with_extension(format!("db.bak-v{SCHEMA_VERSION}"));
        assert!(backup.exists(), "a backup is written before migrating");
        let copy = Connection::open(&backup).unwrap();
        let rows: i64 = copy
            .query_row("SELECT COUNT(*) FROM entries", [], |row| row.get(0))
            .unwrap();
        assert_eq!(rows, 1);
        let version: String = copy
            .query_row(
                "SELECT value FROM meta WHERE key = 'schema_version'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            version,
            SCHEMA_VERSION.to_string(),
            "backup keeps the old schema"
        );
    }

    #[test]
    fn migration_gap_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.db");
        let key = MasterKey::generate();
        drop(open_with(&path, &key).unwrap());
        let db = Database {
            conn: Connection::open(&path).unwrap(),
            cipher: Cipher::new(&key),
        };
        // Target two versions ahead with only one migration: refuse rather
        // than stamp a schema the file does not have.
        let short = [(SCHEMA_VERSION + 1, Migration::Sql("SELECT 1"))];
        let err = db
            .prepare(Some(&path), &short, SCHEMA_VERSION + 2)
            .unwrap_err();
        assert!(matches!(err, Error::Storage(m) if m.contains("no migration path")));
    }

    #[test]
    fn newer_schema_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.db");
        let key = MasterKey::generate();
        {
            let db = open_with(&path, &key).unwrap();
            db.set_schema_version(SCHEMA_VERSION + 50).unwrap();
        }
        let err = open_with(&path, &key).unwrap_err();
        assert!(matches!(err, Error::Storage(m) if m.contains("newer")));
    }

    #[test]
    fn different_key_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.db");
        {
            let db = open_with(&path, &MasterKey::generate()).unwrap();
            insert_one(&db, "sealed with the first key");
        }
        let err = open_with(&path, &MasterKey::generate()).unwrap_err();
        assert!(matches!(err, Error::Storage(m) if m.contains("different master key")));
    }

    #[test]
    fn file_without_fingerprint_is_proven_by_decrypting_a_preview() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.db");
        let right = MasterKey::generate();
        {
            let db = open_with(&path, &right).unwrap();
            insert_one(&db, "written before fingerprints existed");
            db.conn
                .execute("DELETE FROM meta WHERE key = 'key_fingerprint'", [])
                .unwrap();
        }
        assert!(matches!(
            open_with(&path, &MasterKey::generate()).unwrap_err(),
            Error::Storage(_)
        ));
        let db = open_with(&path, &right).unwrap();
        assert_eq!(
            db.meta("key_fingerprint").unwrap().as_deref(),
            Some(db.cipher.fingerprint()),
            "the proven key is bound for next time"
        );
    }

    #[test]
    fn corrupt_file_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.db");
        std::fs::write(&path, b"this is not a database at all, not even close").unwrap();
        assert!(open_with(&path, &MasterKey::generate()).is_err());
    }

    #[test]
    fn version_1_file_gains_deleted_at_through_the_real_migration() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.db");
        let key = MasterKey::generate();
        {
            // Turn a fresh file into what 1.2.0 wrote: no deleted_at column,
            // schema version 1, one entry.
            let db = open_with(&path, &key).unwrap();
            insert_one(&db, "from the old release");
            db.conn
                .execute_batch(
                    "ALTER TABLE entries DROP COLUMN deleted_at;
                     ALTER TABLE entries DROP COLUMN sensitive;",
                )
                .unwrap();
            db.set_schema_version(1).unwrap();
        }
        let db = open_with(&path, &key).unwrap();
        assert_eq!(db.schema_version().unwrap(), SCHEMA_VERSION);
        let deleted_at: i64 = db
            .conn
            .query_row("SELECT deleted_at FROM entries LIMIT 1", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(deleted_at, 0);
        assert_eq!(db.count().unwrap(), 1);
        assert!(path.with_extension("db.bak-v1").exists());
        // The migrated file works end to end: tombstone, restore, purge.
        let id = db.query(&QueryFilter::recent(1)).unwrap()[0].id;
        db.tombstone(id, 500).unwrap();
        assert_eq!(db.count().unwrap(), 0);
        db.restore(id).unwrap();
        assert_eq!(db.count().unwrap(), 1);
    }

    #[test]
    fn restore_lifts_a_tombstone_and_reindexes_the_preview() {
        let db = Database::open_in_memory(Cipher::new(&MasterKey::generate())).unwrap();
        insert_one(&db, "undo candidate zqxtoken");
        let id = db.query(&QueryFilter::recent(1)).unwrap()[0].id;
        db.tombstone(id, 100).unwrap();
        assert!(db
            .query(&QueryFilter {
                search: Some("zqx".into()),
                ..QueryFilter::recent(10)
            })
            .unwrap()
            .is_empty());
        db.restore(id).unwrap();
        let found = db
            .query(&QueryFilter {
                search: Some("zqx".into()),
                ..QueryFilter::recent(10)
            })
            .unwrap();
        assert_eq!(found.len(), 1);
        assert!(!found[0].deleted);
        // Restoring a live entry is a no-op, a purged one is gone for good.
        db.restore(id).unwrap();
        db.tombstone(id, 100).unwrap();
        db.purge_tombstones(200).unwrap();
        assert!(matches!(db.restore(id), Err(Error::NotFound(_))));
    }

    #[test]
    fn purge_uses_the_deletion_time_not_the_copy_time() {
        let db = Database::open_in_memory(Cipher::new(&MasterKey::generate())).unwrap();
        insert_one(&db, "copied long ago");
        let id = db.query(&QueryFilter::recent(1)).unwrap()[0].id;
        // Copied at t=1, deleted at t=1000: a purge of "deleted before 900"
        // must keep it even though the copy is older than that.
        db.tombstone(id, 1000).unwrap();
        assert!(db.purge_tombstones(900).unwrap().is_empty());
        assert!(db.get(id).unwrap().deleted);
        assert!(!db.purge_tombstones(1001).unwrap().is_empty() || db.get(id).is_err());
    }

    fn search(db: &Database, term: &str) -> Vec<i64> {
        db.query(&QueryFilter {
            search: Some(term.into()),
            ..QueryFilter::recent(20)
        })
        .unwrap()
        .into_iter()
        .map(|e| e.id)
        .collect()
    }

    #[test]
    fn indexed_content_is_searchable_beyond_the_preview() {
        let db = Database::open_in_memory(Cipher::new(&MasterKey::generate())).unwrap();
        insert_one(&db, "short preview of a long note");
        let id = db.query(&QueryFilter::recent(1)).unwrap()[0].id;
        assert!(search(&db, "zebra").is_empty());
        db.index_content(id, "lots of words and, at the very end, zebra")
            .unwrap();
        assert_eq!(search(&db, "zebra"), vec![id]);
        assert_eq!(search(&db, "short"), vec![id], "preview still matches");
        // Tombstoned entries leave the index; restore brings the preview
        // back (content is re-indexed when the entry is stored again).
        db.tombstone(id, 5).unwrap();
        assert!(search(&db, "zebra").is_empty());
        db.restore(id).unwrap();
        assert_eq!(search(&db, "short"), vec![id]);
    }

    #[test]
    fn search_folds_case_and_diacritics_including_turkish_dotted_i() {
        let db = Database::open_in_memory(Cipher::new(&MasterKey::generate())).unwrap();
        insert_one(&db, "İstanbul'da yağmur yağıyor");
        let id = db.query(&QueryFilter::recent(1)).unwrap()[0].id;
        assert_eq!(search(&db, "istanbul"), vec![id]);
        assert_eq!(search(&db, "yagmur"), vec![id]);
        assert_eq!(search(&db, "YAĞMUR"), vec![id]);
    }

    #[test]
    fn total_bytes_cap_keeps_the_newest_and_the_pinned() {
        let db = Database::open_in_memory(Cipher::new(&MasterKey::generate())).unwrap();
        let insert = |preview: &str, size: i64, now: i64| -> i64 {
            let hash = crate::storage::crypto::content_hash(preview.as_bytes());
            db.upsert_entry(
                &hash,
                preview,
                ContentKind::Text,
                "text/plain",
                size,
                None,
                Selection::Clipboard,
                now,
                "dev",
                now,
            )
            .unwrap()
        };
        let oldest = insert("oldest", 100, 1);
        let pinned = insert("pinned", 200, 2);
        db.set_pinned(pinned, true).unwrap();
        let newest = insert("newest", 300, 3);
        // 200 (pinned) + 300 fit in 550; the oldest 100 would not.
        assert_eq!(db.enforce_total_bytes(550).unwrap(), vec![oldest]);
        assert!(db.get(oldest).unwrap().deleted);
        assert!(!db.get(newest).unwrap().deleted);
        assert!(!db.get(pinned).unwrap().deleted);
        // Nothing to do when everything fits.
        assert!(db.enforce_total_bytes(550).unwrap().is_empty());
        // Below the pinned size alone, every unpinned entry goes.
        assert_eq!(db.enforce_total_bytes(150).unwrap(), vec![newest]);
        assert!(!db.get(pinned).unwrap().deleted);
    }

    #[test]
    fn maintenance_runs_and_lists_blob_references() {
        let dir = tempfile::tempdir().unwrap();
        let key = MasterKey::generate();
        let db = open_with(&dir.path().join("history.db"), &key).unwrap();
        insert_one(&db, "with a blob");
        let id = db.query(&QueryFilter::recent(1)).unwrap()[0].id;
        db.attach_blob(id, "text/plain", &"a".repeat(64)).unwrap();
        db.attach_blob(id, "text/html", &"b".repeat(64)).unwrap();
        let refs = db.referenced_blobs().unwrap();
        assert!(refs.contains(&"a".repeat(64)));
        assert!(refs.contains(&"b".repeat(64)));
        assert_eq!(refs.len(), 2);
        db.maintain().unwrap();
        assert_eq!(db.query(&QueryFilter::recent(1)).unwrap()[0].id, id);
    }

    #[test]
    fn sensitive_entries_expire_unless_pinned() {
        let db = Database::open_in_memory(Cipher::new(&MasterKey::generate())).unwrap();
        insert_one(&db, "AKIAIOSFODNN7EXAMPLE");
        insert_one(&db, "plain text");
        let entries = db.query(&QueryFilter::recent(10)).unwrap();
        let (plain, secret) = (entries[0].id, entries[1].id);
        db.mark_sensitive(secret).unwrap();
        assert!(db.get(secret).unwrap().sensitive);
        assert!(!db.get(plain).unwrap().sensitive);
        assert!(
            db.expire_sensitive(0).unwrap().is_empty(),
            "not before their time"
        );
        let far_future = i64::MAX / 2;
        assert_eq!(db.expire_sensitive(far_future).unwrap(), vec![secret]);
        assert!(db.get(secret).unwrap().deleted);
        assert!(!db.get(plain).unwrap().deleted);
        // A pinned secret is the user's decision.
        insert_one(&db, "ghp_A1b2C3d4E5f6G7h8I9j0K1l2M3n4O5p6Q7r8");
        let id = db.query(&QueryFilter::recent(1)).unwrap()[0].id;
        db.mark_sensitive(id).unwrap();
        db.set_pinned(id, true).unwrap();
        assert!(db.expire_sensitive(far_future).unwrap().is_empty());
    }

    #[test]
    fn version_3_file_gains_the_sensitive_column() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.db");
        let key = MasterKey::generate();
        {
            let db = open_with(&path, &key).unwrap();
            insert_one(&db, "kept across the column add");
            db.conn
                .execute_batch("ALTER TABLE entries DROP COLUMN sensitive")
                .unwrap();
            db.set_schema_version(3).unwrap();
        }
        let db = open_with(&path, &key).unwrap();
        assert_eq!(db.schema_version().unwrap(), SCHEMA_VERSION);
        assert!(path.with_extension("db.bak-v3").exists());
        let entry = db.query(&QueryFilter::recent(1)).unwrap().remove(0);
        assert!(!entry.sensitive);
        db.mark_sensitive(entry.id).unwrap();
        assert!(db.get(entry.id).unwrap().sensitive);
    }

    #[test]
    fn version_2_index_is_rebuilt_with_the_content_column() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.db");
        let key = MasterKey::generate();
        {
            let db = open_with(&path, &key).unwrap();
            insert_one(&db, "kept across the rebuild");
            // Back to what a version 2 file looked like: the old FTS table
            // without `content`, and the old version stamp.
            db.conn
                .execute_batch(
                    "DROP TABLE entries_fts;
                     CREATE VIRTUAL TABLE entries_fts USING fts5(preview, tokenize='unicode61');",
                )
                .unwrap();
            let id = db.query(&QueryFilter::recent(1)).unwrap()[0].id;
            db.conn
                .execute(
                    "INSERT INTO entries_fts(rowid, preview) VALUES (?1, ?2)",
                    params![id, "kept across the rebuild"],
                )
                .unwrap();
            db.conn
                .execute_batch("ALTER TABLE entries DROP COLUMN sensitive")
                .unwrap();
            db.set_schema_version(2).unwrap();
        }
        let db = open_with(&path, &key).unwrap();
        assert_eq!(db.schema_version().unwrap(), SCHEMA_VERSION);
        assert!(path.with_extension("db.bak-v2").exists());
        let id = db.query(&QueryFilter::recent(1)).unwrap()[0].id;
        assert_eq!(search(&db, "rebuild"), vec![id], "previews were re-added");
        db.index_content(id, "and now content too").unwrap();
        assert_eq!(search(&db, "content"), vec![id]);
    }
}
