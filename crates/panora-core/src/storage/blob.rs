// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! Content-addressed, encrypted blob store.
//!
//! Payloads live outside the database so the DB stays small and fast.
//! Files are named by the BLAKE3 hash of their plaintext content, while the
//! encrypted envelope is bound to that hash with AEAD associated data. Every
//! directory and file is private to the current user on Unix.

use super::crypto::{content_hash, Cipher, ENVELOPE_VERSION};
use crate::error::{Error, Result};
use rand::RngCore;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Encrypted content-addressed file store.
pub struct BlobStore {
    root: PathBuf,
    /// A lock, not a plain field, for the same reason as `Database`'s: only
    /// `rekey` (SEC-01) ever swaps it, never a concurrent access.
    cipher: std::sync::RwLock<Cipher>,
}

impl BlobStore {
    /// Open (creating if needed) the blob store under `root`.
    pub fn open(root: impl AsRef<Path>, cipher: Cipher) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        std::fs::create_dir_all(&root)?;
        restrict_permissions(&root, 0o700)?;
        Ok(Self {
            root,
            cipher: std::sync::RwLock::new(cipher),
        })
    }

    /// Filesystem path for a validated content hash: blobs/<hh>/<hash>.
    fn path_for(&self, hash: &str) -> Result<PathBuf> {
        validate_hash(hash)?;
        Ok(self.root.join(&hash[..2]).join(hash))
    }

    /// Associated data binds the ciphertext to its content-addressed name.
    fn aad(hash: &str) -> Vec<u8> {
        format!("panora/blob/v{ENVELOPE_VERSION}/{hash}").into_bytes()
    }

    /// Store plaintext; returns the content hash. Writing is skipped when an
    /// identical payload already exists (dedup).
    pub fn put(&self, plaintext: &[u8]) -> Result<String> {
        let hash = content_hash(plaintext);
        let path = self.path_for(&hash)?;
        if path.exists() {
            return Ok(hash);
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
            restrict_permissions(parent, 0o700)?;
        }
        let sealed = self
            .cipher
            .read()
            .unwrap()
            .seal_with_aad(&Self::aad(&hash), plaintext)?;
        match Self::write_sealed(&path, &sealed) {
            Ok(()) => Ok(hash),
            // A concurrent writer may have completed the same deduplicated
            // insert; preserve that success instead of surfacing a race.
            Err(_) if path.exists() => Ok(hash),
            Err(e) => Err(e),
        }
    }

    /// Write `sealed` to `path` through a process-specific temporary file
    /// (never following a pre-existing symlink) and an atomic rename — which
    /// on Unix always replaces an existing target — so a reader never
    /// observes a partially written blob, whether this is a fresh write
    /// (`put`) or replacing one under a new key (`rekey`).
    fn write_sealed(path: &Path, sealed: &[u8]) -> Result<()> {
        let mut suffix = [0u8; 8];
        rand::rngs::OsRng.fill_bytes(&mut suffix);
        let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("blob");
        let tmp = path.with_file_name(format!(".{file_name}.tmp-{}", u64::from_le_bytes(suffix)));
        let mut file = OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        restrict_permissions(&tmp, 0o600)?;
        file.write_all(sealed)?;
        file.sync_all()?;
        drop(file);
        if let Err(e) = std::fs::rename(&tmp, path) {
            let _ = std::fs::remove_file(&tmp);
            return Err(e.into());
        }
        Ok(())
    }

    /// Read and decrypt a blob by content hash.
    pub fn get(&self, hash: &str) -> Result<Vec<u8>> {
        let path = self.path_for(hash)?;
        let sealed = std::fs::read(path)?;
        let cipher = self.cipher.read().unwrap();
        match cipher.open_with_aad(&Self::aad(hash), &sealed) {
            Ok(plaintext) => Ok(plaintext),
            Err(error) if !sealed.starts_with(super::crypto::ENVELOPE_MAGIC) => {
                // One-way compatibility for pre-v1 local histories. New
                // writes always use the versioned, context-bound envelope.
                cipher.open(&sealed).map_err(|_| error)
            }
            Err(error) => Err(error),
        }
    }

    /// Re-seal every blob on disk under `new_cipher`, then start using it
    /// for everything from here on (SEC-01).
    ///
    /// Safe to call again after an interruption: each file is opened with
    /// whichever of the current cipher or `new_cipher` can read it (a file
    /// a previous, interrupted attempt already rewrote opens under the new
    /// one and is simply rewritten again, unchanged), and each individual
    /// file is replaced atomically, so a crash never leaves one half old
    /// and half new.
    pub fn rekey(&self, new_cipher: Cipher) -> Result<()> {
        for hash in self.list()? {
            let path = self.path_for(&hash)?;
            let sealed = std::fs::read(&path)?;
            let aad = Self::aad(&hash);
            let plaintext = {
                let old_cipher = self.cipher.read().unwrap();
                old_cipher
                    .open_with_aad(&aad, &sealed)
                    .or_else(|_| old_cipher.open(&sealed))
                    .or_else(|_| new_cipher.open_with_aad(&aad, &sealed))
                    .or_else(|_| new_cipher.open(&sealed))?
            };
            let resealed = new_cipher.seal_with_aad(&aad, &plaintext)?;
            Self::write_sealed(&path, &resealed)?;
        }
        *self.cipher.write().unwrap() = new_cipher;
        Ok(())
    }

    /// Delete a blob by hash. Missing files are not an error.
    pub fn remove(&self, hash: &str) -> Result<()> {
        let path = self.path_for(hash)?;
        if path.exists() {
            std::fs::remove_file(&path)?;
            // Best-effort cleanup of the two-char shard directory.
            if let Some(parent) = path.parent() {
                let _ = std::fs::remove_dir(parent);
            }
        }
        Ok(())
    }

    /// Whether a valid blob exists on disk.
    pub fn exists(&self, hash: &str) -> bool {
        self.path_for(hash)
            .map(|path| path.exists())
            .unwrap_or(false)
    }

    /// Total size of all blobs in bytes (for settings UI / benchmarks).
    pub fn total_size(&self) -> u64 {
        walk_size(&self.root)
    }

    /// Every blob hash on disk, for the orphan scan. A temporary file left
    /// by an interrupted write is removed once it is an hour old.
    pub fn list(&self) -> Result<Vec<String>> {
        let mut hashes = Vec::new();
        for shard in std::fs::read_dir(&self.root)? {
            let shard = shard?;
            if !shard.file_type()?.is_dir() {
                continue;
            }
            for file in std::fs::read_dir(shard.path())? {
                let file = file?;
                let name = file.file_name();
                let name = name.to_string_lossy();
                if validate_hash(&name).is_ok() {
                    hashes.push(name.into_owned());
                } else if name.starts_with('.') && name.contains(".tmp-") {
                    let stale = file
                        .metadata()
                        .and_then(|m| m.modified())
                        .map(|t| {
                            t.elapsed().unwrap_or_default() > std::time::Duration::from_secs(3600)
                        })
                        .unwrap_or(false);
                    if stale {
                        let _ = std::fs::remove_file(file.path());
                    }
                }
            }
        }
        Ok(hashes)
    }
}

fn validate_hash(hash: &str) -> Result<()> {
    if hash.len() != 64
        || !hash
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(Error::Config("invalid blob content hash".into()));
    }
    Ok(())
}

fn restrict_permissions(path: &Path, mode: u32) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))?;
    }
    #[cfg(not(unix))]
    let _ = (path, mode);
    Ok(())
}

fn walk_size(dir: &Path) -> u64 {
    let mut total = 0;
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                total += walk_size(&path);
            } else if let Ok(meta) = path.metadata() {
                total += meta.len();
            }
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::crypto::MasterKey;

    fn store(dir: &tempfile::TempDir) -> BlobStore {
        BlobStore::open(
            dir.path().join("blobs"),
            Cipher::new(&MasterKey::generate()),
        )
        .unwrap()
    }

    #[test]
    fn put_get_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(&dir);
        let data = b"binary payload \x00\x01\x02";
        let hash = store.put(data).unwrap();
        assert_eq!(store.get(&hash).unwrap(), data);
        assert!(store.exists(&hash));
    }

    #[test]
    fn invalid_hash_is_rejected_without_path_traversal() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(&dir);
        assert!(!store.exists("../../etc/passwd"));
        assert!(store.get("../../etc/passwd").is_err());
        assert!(store.remove("bad").is_err());
    }

    #[test]
    fn dedup_single_file() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(&dir);
        let h1 = store.put(b"same content").unwrap();
        let h2 = store.put(b"same content").unwrap();
        assert_eq!(h1, h2);
        let count = walkdir_count(dir.path());
        assert_eq!(count, 1, "identical payloads must share one file");
    }

    #[test]
    fn stored_bytes_are_encrypted_and_versioned() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(&dir);
        let plaintext = b"top secret clipboard";
        let hash = store.put(plaintext).unwrap();
        let raw = std::fs::read(store.path_for(&hash).unwrap()).unwrap();
        assert!(raw.starts_with(crate::storage::crypto::ENVELOPE_MAGIC));
        assert_eq!(
            raw[crate::storage::crypto::ENVELOPE_MAGIC.len()],
            ENVELOPE_VERSION
        );
        assert_ne!(
            &raw[crate::storage::crypto::ENVELOPE_HEADER_LEN + 24..],
            plaintext,
            "file on disk must not contain plaintext"
        );
        assert!(!raw
            .windows(plaintext.len())
            .any(|w| w == plaintext.as_slice()));
    }

    #[cfg(unix)]
    #[test]
    fn store_and_blob_permissions_are_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let store = store(&dir);
        let hash = store.put(b"permissions").unwrap();
        let root_mode = std::fs::metadata(&store.root).unwrap().permissions().mode() & 0o777;
        let file_mode = std::fs::metadata(store.path_for(&hash).unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(root_mode, 0o700);
        assert_eq!(file_mode, 0o600);
    }

    #[test]
    fn remove_works_and_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(&dir);
        let hash = store.put(b"to delete").unwrap();
        assert!(store.exists(&hash));
        store.remove(&hash).unwrap();
        assert!(!store.exists(&hash));
        store.remove(&hash).unwrap();
    }

    #[test]
    fn get_missing_is_error() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(&dir);
        assert!(store.get("ab".repeat(32).as_str()).is_err());
    }

    #[test]
    fn total_size_tracks_files() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(&dir);
        assert_eq!(store.total_size(), 0);
        store.put(&vec![7u8; 1000]).unwrap();
        assert!(store.total_size() > 1000);
    }

    fn walkdir_count(dir: &Path) -> usize {
        let mut count = 0;
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let p = entry.path();
            if p.is_dir() {
                count += walkdir_count(&p);
            } else {
                count += 1;
            }
        }
        count
    }

    // --- SEC-01: master key rotation ---

    #[test]
    fn rekey_reseals_every_blob_and_old_key_stops_working() {
        let dir = tempfile::tempdir().unwrap();
        let old_key = MasterKey::generate();
        let store = BlobStore::open(dir.path().join("blobs"), Cipher::new(&old_key)).unwrap();
        let a = store.put(b"first blob").unwrap();
        let b = store.put(b"second blob, a bit longer").unwrap();

        let new_key = MasterKey::generate();
        store.rekey(Cipher::new(&new_key)).unwrap();

        assert_eq!(store.get(&a).unwrap(), b"first blob");
        assert_eq!(store.get(&b).unwrap(), b"second blob, a bit longer");

        // A fresh handle opened under the old key can no longer read it.
        let reopened = BlobStore::open(dir.path().join("blobs"), Cipher::new(&old_key)).unwrap();
        assert!(reopened.get(&a).is_err());
    }

    #[test]
    fn rekey_is_idempotent_so_an_interrupted_rotation_can_resume() {
        let dir = tempfile::tempdir().unwrap();
        let old_key = MasterKey::generate();
        let store = BlobStore::open(dir.path().join("blobs"), Cipher::new(&old_key)).unwrap();
        let hash = store.put(b"resumed rotation payload").unwrap();
        let new_key = MasterKey::generate();

        store.rekey(Cipher::new(&new_key)).unwrap();
        // The "resume": panod would call this again after restarting with
        // the same pending key, having found `rotation_state` still set.
        store.rekey(Cipher::new(&new_key)).unwrap();

        assert_eq!(store.get(&hash).unwrap(), b"resumed rotation payload");
    }
}
