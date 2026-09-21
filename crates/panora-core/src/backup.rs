// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! Encrypted export/import archive format (CLI-03).
//!
//! `panora-cli export` writes one file: a magic, a format version, a
//! random salt, then an XChaCha20-Poly1305 envelope (reusing
//! [`crate::storage::crypto::Cipher`]) sealing a tar archive built from
//! `entries.json` (metadata for every entry) plus `blobs/<hash>` (each
//! entry's decrypted payload bytes, content-addressed the same way
//! [`crate::storage::BlobStore`] names them on disk, so a payload shared
//! by more than one entry is only stored once).
//!
//! The key is derived straight from the passphrase with Argon2id
//! ([`crate::lock::derive_key`]); there is no separate stored verifier the
//! way [`crate::lock::LockSecret`] keeps one; the AEAD tag failing to
//! authenticate on `open_archive` already answers "wrong passphrase, or
//! the archive is corrupt" — a second check would only duplicate that.

use crate::error::{Error, Result};
use crate::lock::derive_key;
use crate::model::MimePayload;
use crate::storage::crypto::{content_hash, Cipher, MasterKey};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Read;

/// Marks a file as a Panora backup archive, distinct from anything else a
/// user might point `import` at by mistake.
const MAGIC: &[u8; 4] = b"PNRB";
/// On-disk archive format version.
const FORMAT_VERSION: u8 = 1;
/// Length of the random salt used to derive the archive's key.
const SALT_LEN: usize = 16;
/// Fixed header length before the sealed tar begins.
const HEADER_LEN: usize = MAGIC.len() + 1 + SALT_LEN;

/// One entry's metadata inside the archive. Deliberately not
/// [`crate::model::Entry`] itself: that carries a database row id, which
/// means nothing on import, and no payload references at all.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupEntry {
    /// Content hash of the entry's payloads combined, as `panod` computes
    /// it on capture — carried through so import can log/debug against it,
    /// though re-insertion recomputes and trusts its own, not this.
    pub content_hash: String,
    /// Preview text exactly as stored (already masked, if it was).
    pub preview: String,
    /// `ContentKind::as_str()`.
    pub kind: String,
    /// Primary MIME type (first payload).
    pub primary_mime: String,
    /// Total payload size in bytes.
    pub size_bytes: i64,
    /// Best-effort source application name.
    pub source_app: Option<String>,
    /// `Selection::as_str()`.
    pub selection: String,
    /// Unix timestamp (seconds) of first capture.
    pub created_at: i64,
    /// Unix timestamp of most recent capture.
    pub last_seen_at: i64,
    /// Whether the entry was pinned.
    pub pinned: bool,
    /// Whether the entry was flagged as looking like a secret.
    pub sensitive: bool,
    /// (mime, blob content hash) pairs, in the entry's preferred order.
    /// The bytes for each hash live at `blobs/<hash>` in the tar.
    pub payloads: Vec<(String, String)>,
}

/// Build the encrypted archive for `entries`, each paired with its
/// already-decrypted payload bytes in the same order as its
/// `BackupEntry::payloads`.
pub fn build_archive(
    entries: &[(BackupEntry, Vec<MimePayload>)],
    passphrase: &str,
) -> Result<Vec<u8>> {
    let mut tar = tar::Builder::new(Vec::new());

    let manifest: Vec<&BackupEntry> = entries.iter().map(|(e, _)| e).collect();
    let manifest_json =
        serde_json::to_vec_pretty(&manifest).map_err(|e| Error::Storage(e.to_string()))?;
    append_tar_file(&mut tar, "entries.json", &manifest_json)?;

    let mut written = std::collections::HashSet::new();
    for (entry, payloads) in entries {
        for ((_, hash), payload) in entry.payloads.iter().zip(payloads) {
            if written.insert(hash.clone()) {
                append_tar_file(&mut tar, &format!("blobs/{hash}"), &payload.data)?;
            }
        }
    }
    let tar_bytes = tar
        .into_inner()
        .map_err(|e| Error::Storage(format!("building the backup archive: {e}")))?;

    let mut salt = [0u8; SALT_LEN];
    rand::rngs::OsRng.fill_bytes(&mut salt);
    let cipher = Cipher::new(&MasterKey::from_bytes(derive_key(passphrase, &salt)?));
    let sealed = cipher.seal(&tar_bytes)?;

    let mut out = Vec::with_capacity(HEADER_LEN + sealed.len());
    out.extend_from_slice(MAGIC);
    out.push(FORMAT_VERSION);
    out.extend_from_slice(&salt);
    out.extend_from_slice(&sealed);
    Ok(out)
}

/// Decrypt and unpack an archive built by [`build_archive`]. Every blob's
/// bytes are checked against the hash `entries.json` says they should
/// have, so a truncated or tampered archive is refused rather than
/// silently importing the wrong payload under an entry.
pub fn open_archive(
    passphrase: &str,
    bytes: &[u8],
) -> Result<Vec<(BackupEntry, Vec<MimePayload>)>> {
    if bytes.len() < HEADER_LEN || &bytes[..MAGIC.len()] != MAGIC {
        return Err(Error::Storage("not a Panora backup archive".into()));
    }
    let version = bytes[MAGIC.len()];
    if version != FORMAT_VERSION {
        return Err(Error::Storage(format!(
            "backup archive format version {version} is not supported by this build"
        )));
    }
    let salt = &bytes[MAGIC.len() + 1..HEADER_LEN];
    let sealed = &bytes[HEADER_LEN..];
    let cipher = Cipher::new(&MasterKey::from_bytes(derive_key(passphrase, salt)?));
    let tar_bytes = cipher
        .open(sealed)
        .map_err(|_| Error::Storage("wrong passphrase, or the archive is corrupt".into()))?;

    let mut archive = tar::Archive::new(&tar_bytes[..]);
    let mut manifest: Option<Vec<BackupEntry>> = None;
    let mut blobs: HashMap<String, Vec<u8>> = HashMap::new();
    for file in archive
        .entries()
        .map_err(|e| Error::Storage(format!("reading the backup archive: {e}")))?
    {
        let mut file =
            file.map_err(|e| Error::Storage(format!("reading the backup archive: {e}")))?;
        let path = file
            .path()
            .map_err(|e| Error::Storage(e.to_string()))?
            .to_string_lossy()
            .into_owned();
        let mut data = Vec::new();
        file.read_to_end(&mut data)
            .map_err(|e| Error::Storage(e.to_string()))?;
        if path == "entries.json" {
            manifest =
                Some(serde_json::from_slice(&data).map_err(|e| Error::Storage(e.to_string()))?);
        } else if let Some(hash) = path.strip_prefix("blobs/") {
            blobs.insert(hash.to_string(), data);
        }
    }
    let manifest =
        manifest.ok_or_else(|| Error::Storage("backup archive has no entries.json".into()))?;

    let mut out = Vec::with_capacity(manifest.len());
    for entry in manifest {
        let mut payloads = Vec::with_capacity(entry.payloads.len());
        for (mime, hash) in &entry.payloads {
            let data = blobs
                .get(hash)
                .ok_or_else(|| Error::Storage(format!("backup archive is missing blob {hash}")))?
                .clone();
            if content_hash(&data) != *hash {
                return Err(Error::Storage(format!(
                    "backup archive blob {hash} failed its integrity check"
                )));
            }
            payloads.push(MimePayload::new(mime.clone(), data));
        }
        out.push((entry, payloads));
    }
    Ok(out)
}

fn append_tar_file(tar: &mut tar::Builder<Vec<u8>>, name: &str, data: &[u8]) -> Result<()> {
    let mut header = tar::Header::new_gnu();
    header.set_size(data.len() as u64);
    header.set_mode(0o600);
    header.set_cksum();
    tar.append_data(&mut header, name, data)
        .map_err(|e| Error::Storage(format!("writing {name} into the backup archive: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<(BackupEntry, Vec<MimePayload>)> {
        let text = b"exported clipboard text".to_vec();
        let image = vec![0x89u8; 64];
        vec![
            (
                BackupEntry {
                    content_hash: "irrelevant-on-import".into(),
                    preview: "exported clipboard text".into(),
                    kind: "text".into(),
                    primary_mime: "text/plain".into(),
                    size_bytes: text.len() as i64,
                    source_app: Some("test".into()),
                    selection: "clipboard".into(),
                    created_at: 1,
                    last_seen_at: 2,
                    pinned: true,
                    sensitive: false,
                    payloads: vec![("text/plain".into(), content_hash(&text))],
                },
                vec![MimePayload::new("text/plain", text.clone())],
            ),
            (
                BackupEntry {
                    content_hash: "irrelevant-on-import-2".into(),
                    preview: "[image]".into(),
                    kind: "image".into(),
                    primary_mime: "image/png".into(),
                    size_bytes: image.len() as i64,
                    source_app: None,
                    selection: "clipboard".into(),
                    created_at: 3,
                    last_seen_at: 4,
                    pinned: false,
                    sensitive: false,
                    payloads: vec![("image/png".into(), content_hash(&image))],
                },
                vec![MimePayload::new("image/png", image.clone())],
            ),
        ]
    }

    #[test]
    fn round_trips_entries_and_payloads() {
        let entries = sample();
        let archive = build_archive(&entries, "correct horse battery staple").unwrap();
        let opened = open_archive("correct horse battery staple", &archive).unwrap();

        assert_eq!(opened.len(), 2);
        let (first, first_payloads) = &opened[0];
        assert_eq!(first.preview, "exported clipboard text");
        assert!(first.pinned);
        assert_eq!(first_payloads[0].data, b"exported clipboard text");
        let (second, second_payloads) = &opened[1];
        assert_eq!(second.kind, "image");
        assert_eq!(second_payloads[0].data, vec![0x89u8; 64]);
    }

    #[test]
    fn wrong_passphrase_is_refused() {
        let archive = build_archive(&sample(), "right passphrase").unwrap();
        assert!(open_archive("wrong passphrase", &archive).is_err());
    }

    #[test]
    fn a_shared_payload_is_stored_once() {
        let text = b"shared between two entries".to_vec();
        let hash = content_hash(&text);
        let entries = vec![
            (
                BackupEntry {
                    content_hash: "a".into(),
                    preview: "one".into(),
                    kind: "text".into(),
                    primary_mime: "text/plain".into(),
                    size_bytes: text.len() as i64,
                    source_app: None,
                    selection: "clipboard".into(),
                    created_at: 1,
                    last_seen_at: 1,
                    pinned: false,
                    sensitive: false,
                    payloads: vec![("text/plain".into(), hash.clone())],
                },
                vec![MimePayload::new("text/plain", text.clone())],
            ),
            (
                BackupEntry {
                    content_hash: "b".into(),
                    preview: "two".into(),
                    kind: "text".into(),
                    primary_mime: "text/plain".into(),
                    size_bytes: text.len() as i64,
                    source_app: None,
                    selection: "primary".into(),
                    created_at: 2,
                    last_seen_at: 2,
                    pinned: false,
                    sensitive: false,
                    payloads: vec![("text/plain".into(), hash)],
                },
                vec![MimePayload::new("text/plain", text)],
            ),
        ];
        let archive = build_archive(&entries, "pw").unwrap();
        let opened = open_archive("pw", &archive).unwrap();
        assert_eq!(opened.len(), 2);
        assert_eq!(opened[0].1[0].data, opened[1].1[0].data);

        // The tar itself should only carry the blob once, not twice.
        let salt_end = MAGIC.len() + 1 + SALT_LEN;
        let cipher = Cipher::new(&MasterKey::from_bytes(
            derive_key("pw", &archive[MAGIC.len() + 1..salt_end]).unwrap(),
        ));
        let tar_bytes = cipher.open(&archive[salt_end..]).unwrap();
        let mut count = 0;
        let mut inner = tar::Archive::new(&tar_bytes[..]);
        for file in inner.entries().unwrap() {
            let file = file.unwrap();
            if file.path().unwrap().starts_with("blobs/") {
                count += 1;
            }
        }
        assert_eq!(count, 1, "a payload shared by two entries is stored once");
    }

    #[test]
    fn tampered_archive_is_rejected() {
        let mut archive = build_archive(&sample(), "pw").unwrap();
        let last = archive.len() - 1;
        archive[last] ^= 0x01;
        assert!(open_archive("pw", &archive).is_err());
    }

    #[test]
    fn not_an_archive_at_all_is_rejected() {
        assert!(open_archive("pw", b"just some random bytes, not a backup").is_err());
    }
}
