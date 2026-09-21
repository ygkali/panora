// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! Second-layer lock (SEC-02): a password gate on top of the master key
//! that is always loaded and ready, so a *running* daemon can refuse to
//! hand out clipboard history to anyone who does not know it.
//!
//! ## Threat model
//!
//! This is deliberately a different layer than the OS keyring
//! ([`crate::storage::MasterKey`]/[`crate::storage::Cipher`]), which
//! protects the key at rest and must stay usable **unattended** — `panod`
//! is a systemd user service that has to survive a reboot with no one
//! there to type anything. Setting a lock password never removes the
//! plain master key from the keyring, so:
//!
//! - It *does* stop someone with desktop/IPC access to an already-running,
//!   locked daemon from listing, previewing or recalling history through
//!   `panora-cli`/the popup: [`LockSecret::verify`] gates those requests.
//! - It does *not* stop someone who can read the Secret Service keyring
//!   item directly (a keyring compromise, not just desktop access) — that
//!   person still has the plain key regardless of the lock. [`LockSecret::
//!   wrap`] additionally produces a password-gated **backup** copy of the
//!   master key for the keyring, so a lost or corrupted plain item still
//!   has a recovery path for someone who remembers the lock password, but
//!   this backup is not part of the daemon's normal unlock flow.
//!
//! ## Key derivation
//!
//! The verifier and the KEK are deliberately derived with *independent*
//! Argon2id salts. A PHC hash string is designed to be safe to store for
//! verification, but its encoded output **is** the raw hash bytes — reusing
//! it as key material would let anyone who can read the stored verifier
//! (which is not treated as secret) skip straight to the KEK without ever
//! knowing the password.

use crate::error::{Error, Result};
use crate::storage::crypto::{hex_decode, hex_encode};
use crate::storage::{Cipher, MasterKey};
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use rand::RngCore;

/// Length of the independent salt used for KEK derivation.
const KEK_SALT_LEN: usize = 16;

/// What gets stored (in the database's `meta` table, next to
/// `key_fingerprint`) to check future unlock attempts and re-derive the
/// KEK. Safe to keep there in the clear: a PHC hash is meant to be
/// persisted, and the KEK salt is not secret, only independent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockSecret {
    /// PHC-encoded Argon2id hash, checked by `verify`.
    pub verifier: String,
    /// Hex-encoded salt for `derive_kek`.
    pub kek_salt: String,
}

impl LockSecret {
    /// Set up a new lock password: a verifier for `verify`, and the salt
    /// `derive_kek` will use. Call once when the user sets or changes the
    /// password.
    pub fn new(password: &str) -> Result<Self> {
        let verifier_salt = SaltString::generate(&mut rand::rngs::OsRng);
        let verifier = Argon2::default()
            .hash_password(password.as_bytes(), &verifier_salt)
            .map_err(|_| Error::Crypto)?
            .to_string();
        let mut kek_salt = [0u8; KEK_SALT_LEN];
        rand::rngs::OsRng.fill_bytes(&mut kek_salt);
        Ok(Self {
            verifier,
            kek_salt: hex_encode(&kek_salt),
        })
    }

    /// Check a candidate password against the stored verifier.
    pub fn verify(&self, password: &str) -> bool {
        let Ok(hash) = PasswordHash::new(&self.verifier) else {
            return false;
        };
        Argon2::default()
            .verify_password(password.as_bytes(), &hash)
            .is_ok()
    }

    /// Derive the key-encryption-key for `password`. Independent of the
    /// verifier's own salt (see the module docs on why that matters).
    fn derive_kek(&self, password: &str) -> Result<[u8; 32]> {
        let salt = hex_decode(&self.kek_salt).ok_or(Error::Crypto)?;
        let mut kek = [0u8; 32];
        Argon2::default()
            .hash_password_into(password.as_bytes(), &salt, &mut kek)
            .map_err(|_| Error::Crypto)?;
        Ok(kek)
    }

    /// Wrap `key` under `password`'s KEK, for password-gated backup storage
    /// in the keyring.
    pub fn wrap(&self, password: &str, key: &MasterKey) -> Result<Vec<u8>> {
        // `MasterKey` zeroizes on drop, so wrapping the KEK bytes in one
        // immediately (rather than holding them as a bare array) is enough
        // to scrub them once this temporary goes out of scope.
        let cipher = Cipher::new(&MasterKey::from_bytes(self.derive_kek(password)?));
        cipher.seal(key.as_bytes())
    }

    /// Unwrap a backup produced by `wrap`. Fails on the wrong password the
    /// same way any AEAD open does on the wrong key: the tag simply does
    /// not verify.
    pub fn unwrap_key(&self, password: &str, wrapped: &[u8]) -> Result<MasterKey> {
        let cipher = Cipher::new(&MasterKey::from_bytes(self.derive_kek(password)?));
        let bytes = cipher.open(wrapped)?;
        let bytes: [u8; 32] = bytes.try_into().map_err(|_| Error::Crypto)?;
        Ok(MasterKey::from_bytes(bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_accepts_the_right_password_and_rejects_others() {
        let secret = LockSecret::new("correct horse battery staple").unwrap();
        assert!(secret.verify("correct horse battery staple"));
        assert!(!secret.verify("wrong password"));
    }

    #[test]
    fn wrap_unwrap_roundtrips_and_rejects_the_wrong_password() {
        let secret = LockSecret::new("hunter2").unwrap();
        let key = MasterKey::generate();
        let wrapped = secret.wrap("hunter2", &key).unwrap();
        let unwrapped = secret.unwrap_key("hunter2", &wrapped).unwrap();
        assert_eq!(unwrapped.as_bytes(), key.as_bytes());
        assert!(secret.unwrap_key("wrong", &wrapped).is_err());
    }

    #[test]
    fn verifier_bytes_never_equal_the_kek() {
        // The whole point of the independent salt: the stored, "safe to
        // keep" verifier must never itself be usable as the KEK.
        let secret = LockSecret::new("password").unwrap();
        let kek = secret.derive_kek("password").unwrap();
        assert!(!secret.verifier.as_bytes().windows(32).any(|w| w == kek));
    }

    #[test]
    fn two_setups_never_reuse_a_salt() {
        let a = LockSecret::new("same password").unwrap();
        let b = LockSecret::new("same password").unwrap();
        assert_ne!(a.verifier, b.verifier);
        assert_ne!(a.kek_salt, b.kek_salt);
    }
}
