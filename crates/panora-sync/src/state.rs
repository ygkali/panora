// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! What a device keeps between runs: its identity key, its name and its
//! group, in one file sealed with a key from the Secret Service (the
//! `panora-sync` process fetches it there, as `panod` does its master key).

use crate::bytes::b64_secret_vec;
use crate::error::{Error, Result};
use crate::group::{validate_name, GroupState};
use crate::identity::DeviceIdentity;
use panora_core::storage::{Cipher, MasterKey};
use serde::{Deserialize, Serialize};
use std::path::Path;
use zeroize::Zeroizing;

/// State file format version.
const STATE_VERSION: u32 = 1;
/// Associated data binding the ciphertext to its purpose.
const STATE_AAD: &[u8] = b"panora-sync-state/1";

/// A device's persistent sync state.
#[derive(Debug)]
pub struct SyncState {
    /// Long-term identity key.
    pub identity: DeviceIdentity,
    /// The name this device gives itself when pairing.
    pub device_name: String,
    /// The group, once paired or created.
    pub group: Option<GroupState>,
}

#[derive(Serialize, Deserialize)]
struct Stored {
    version: u32,
    #[serde(with = "b64_secret_vec")]
    identity: Zeroizing<Vec<u8>>,
    device_name: String,
    group: Option<GroupState>,
}

impl SyncState {
    /// A new device: fresh identity, no group.
    pub fn new(device_name: &str) -> Result<Self> {
        validate_name(device_name)?;
        Ok(Self {
            identity: DeviceIdentity::generate()?,
            device_name: device_name.to_string(),
            group: None,
        })
    }

    /// Load the state from `path`; `None` if the file does not exist. A
    /// file sealed with another key, altered, or holding a roster that does
    /// not verify is an error, never silently replaced.
    pub fn load(path: &Path, key: &MasterKey) -> Result<Option<Self>> {
        let sealed = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let json = Zeroizing::new(Cipher::new(key).open_with_aad(STATE_AAD, &sealed)?);
        let stored: Stored = serde_json::from_slice(&json)?;
        if stored.version != STATE_VERSION {
            return Err(Error::Protocol("sync state file has an unknown version"));
        }
        validate_name(&stored.device_name)?;
        let identity = DeviceIdentity::from_pkcs8(&stored.identity)?;
        if let Some(group) = &stored.group {
            if group.me() != identity.public() {
                return Err(Error::Roster(
                    "the stored group belongs to another identity",
                ));
            }
        }
        Ok(Some(Self {
            identity,
            device_name: stored.device_name,
            group: stored.group,
        }))
    }

    /// Seal and write the state to `path` atomically, readable by the owner
    /// only.
    pub fn save(&self, path: &Path, key: &MasterKey) -> Result<()> {
        let stored = Stored {
            version: STATE_VERSION,
            identity: Zeroizing::new(self.identity.pkcs8().to_vec()),
            device_name: self.device_name.clone(),
            group: self.group.clone(),
        };
        let json = Zeroizing::new(serde_json::to_vec(&stored)?);
        let sealed = Cipher::new(key).seal_with_aad(STATE_AAD, &json)?;
        let dir = path
            .parent()
            .ok_or(Error::Protocol("state path has no directory"))?;
        std::fs::create_dir_all(dir)?;
        let tmp = path.with_extension("tmp");
        write_private(&tmp, &sealed)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }
}

#[cfg(unix)]
fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    // A leftover temporary file could carry looser permissions (the mode
    // applies only on creation) or be a symlink; start from a fresh file.
    match std::fs::remove_file(path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
        _ => {}
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

#[cfg(not(unix))]
fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    Ok(std::fs::write(path, bytes)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_load_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sync").join("state.bin");
        let key = MasterKey::generate();
        assert!(SyncState::load(&path, &key).unwrap().is_none());

        let mut state = SyncState::new("laptop").unwrap();
        state.group = Some(
            GroupState::create(
                &state.identity,
                "0123456789abcdef0123456789abcdef",
                "laptop",
                1,
            )
            .unwrap(),
        );
        state.save(&path, &key).unwrap();
        let back = SyncState::load(&path, &key).unwrap().unwrap();
        assert_eq!(back.identity.public(), state.identity.public());
        assert_eq!(back.device_name, "laptop");
        let (a, b) = (back.group.unwrap(), state.group.unwrap());
        assert_eq!(a.current().hash(), b.current().hash());
        assert!(a.current_key().is_some());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let raw = std::fs::read(&path).unwrap();
        assert!(!raw.windows(6).any(|w| w == b"laptop"));
    }

    #[cfg(unix)]
    #[test]
    fn a_leftover_temporary_file_does_not_loosen_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.bin");
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, b"old").unwrap();
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o644)).unwrap();
        let key = MasterKey::generate();
        SyncState::new("desk").unwrap().save(&path, &key).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        assert!(SyncState::load(&path, &key).unwrap().is_some());
    }

    #[test]
    fn wrong_key_or_tampering_fails_loudly() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.bin");
        let key = MasterKey::generate();
        SyncState::new("desk").unwrap().save(&path, &key).unwrap();
        assert!(SyncState::load(&path, &MasterKey::generate()).is_err());
        let mut raw = std::fs::read(&path).unwrap();
        let last = raw.len() - 1;
        raw[last] ^= 1;
        std::fs::write(&path, raw).unwrap();
        assert!(SyncState::load(&path, &key).is_err());
    }
}
