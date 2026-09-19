// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! Versioned XChaCha20-Poly1305 encryption helpers (ADR 0003).
//!
//! New ciphertexts use the envelope `[version][24-byte nonce][ciphertext+tag]`.
//! A fresh random 192-bit nonce is generated for every seal. The optional
//! associated data binds a ciphertext to its storage context without being
//! encrypted. The reader still accepts the pre-versioned `[nonce][ciphertext]`
//! format so existing local histories can be opened and migrated on write.
//! The 32-byte master key lives in the OS keyring and zeroizes on drop.

use crate::error::{Error, Result};
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use rand::RngCore;
use zeroize::{Zeroize, ZeroizeOnDrop};

/// Magic prefix for the versioned Panora ciphertext envelope.
pub const ENVELOPE_MAGIC: &[u8; 4] = b"PNR1";
/// Current on-disk envelope version.
pub const ENVELOPE_VERSION: u8 = 1;
/// Versioned envelope header length (`magic + version`).
pub const ENVELOPE_HEADER_LEN: usize = ENVELOPE_MAGIC.len() + 1;
/// Length of the master key in bytes.
pub const KEY_LEN: usize = 32;
/// Length of the XChaCha20 nonce in bytes.
pub const NONCE_LEN: usize = 24;
/// Poly1305 authentication tag length.
pub const TAG_LEN: usize = 16;

/// The 32-byte master encryption key. Zeroized when dropped.
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct MasterKey {
    bytes: [u8; KEY_LEN],
}

impl MasterKey {
    /// Generate a fresh random key (first run or explicit rotation).
    pub fn generate() -> Self {
        let mut bytes = [0u8; KEY_LEN];
        rand::rngs::OsRng.fill_bytes(&mut bytes);
        Self { bytes }
    }

    /// Wrap existing key material loaded from the keyring.
    pub fn from_bytes(bytes: [u8; KEY_LEN]) -> Self {
        Self { bytes }
    }

    /// Borrow raw key bytes only for the Secret Service store operation.
    pub fn as_bytes(&self) -> &[u8; KEY_LEN] {
        &self.bytes
    }
}

impl std::fmt::Debug for MasterKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MasterKey([REDACTED])")
    }
}

/// Stateless AEAD cipher bound to a master key.
pub struct Cipher {
    aead: XChaCha20Poly1305,
    fingerprint: String,
}

impl Cipher {
    /// Build a cipher from a master key reference.
    pub fn new(key: &MasterKey) -> Self {
        Self {
            aead: XChaCha20Poly1305::new_from_slice(key.as_bytes())
                .expect("32-byte key is always valid for XChaCha20Poly1305"),
            fingerprint: key_fingerprint(key.as_bytes()),
        }
    }

    /// Short public identifier of the key this cipher uses.
    ///
    /// A keyed BLAKE3 derivation, so it reveals nothing about the key but
    /// changes with it; the database stores it to notice when it is opened
    /// with a key other than the one that encrypted it.
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }

    /// Encrypt plaintext with no associated data.
    pub fn seal(&self, plaintext: &[u8]) -> Result<Vec<u8>> {
        self.seal_with_aad(&[], plaintext)
    }

    /// Encrypt plaintext and bind it to a non-secret storage context.
    pub fn seal_with_aad(&self, aad: &[u8], plaintext: &[u8]) -> Result<Vec<u8>> {
        let mut nonce_bytes = [0u8; NONCE_LEN];
        rand::rngs::OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = XNonce::from_slice(&nonce_bytes);
        let ciphertext = self
            .aead
            .encrypt(
                nonce,
                chacha20poly1305::aead::Payload {
                    msg: plaintext,
                    aad,
                },
            )
            .map_err(Error::from)?;
        let mut out = Vec::with_capacity(ENVELOPE_HEADER_LEN + NONCE_LEN + ciphertext.len());
        out.extend_from_slice(ENVELOPE_MAGIC);
        out.push(ENVELOPE_VERSION);
        out.extend_from_slice(&nonce_bytes);
        out.extend_from_slice(&ciphertext);
        Ok(out)
    }

    /// Decrypt a versioned or legacy ciphertext with no associated data.
    pub fn open(&self, sealed: &[u8]) -> Result<Vec<u8>> {
        self.open_with_aad(&[], sealed)
    }

    /// Decrypt a versioned ciphertext bound to `aad`.
    ///
    /// Legacy ciphertexts are accepted only when no AAD is requested. This
    /// keeps existing databases readable while preventing callers from
    /// accidentally treating an unbound legacy blob as a context-bound one.
    pub fn open_with_aad(&self, aad: &[u8], sealed: &[u8]) -> Result<Vec<u8>> {
        if sealed.starts_with(ENVELOPE_MAGIC) {
            if sealed.len() < ENVELOPE_HEADER_LEN + NONCE_LEN + TAG_LEN
                || sealed[ENVELOPE_MAGIC.len()] != ENVELOPE_VERSION
            {
                return Err(Error::Crypto);
            }
            let (nonce_bytes, ciphertext) = sealed[ENVELOPE_HEADER_LEN..].split_at(NONCE_LEN);
            let nonce = XNonce::from_slice(nonce_bytes);
            return self
                .aead
                .decrypt(
                    nonce,
                    chacha20poly1305::aead::Payload {
                        msg: ciphertext,
                        aad,
                    },
                )
                .map_err(Error::from);
        }
        if !aad.is_empty() || sealed.len() < NONCE_LEN + TAG_LEN {
            return Err(Error::Crypto);
        }
        let (nonce_bytes, ciphertext) = sealed.split_at(NONCE_LEN);
        let nonce = XNonce::from_slice(nonce_bytes);
        self.aead.decrypt(nonce, ciphertext).map_err(Error::from)
    }

    /// Encrypt a UTF-8 string, returning hex-encoded versioned ciphertext.
    pub fn seal_text(&self, text: &str) -> Result<String> {
        Ok(hex_encode(&self.seal(text.as_bytes())?))
    }

    /// Encrypt UTF-8 text and bind it to a storage context.
    pub fn seal_text_with_aad(&self, aad: &[u8], text: &str) -> Result<String> {
        Ok(hex_encode(&self.seal_with_aad(aad, text.as_bytes())?))
    }

    /// Decrypt versioned or legacy hex-encoded preview text.
    pub fn open_text(&self, sealed_hex: &str) -> Result<String> {
        let sealed = hex_decode(sealed_hex).ok_or(Error::Crypto)?;
        let plaintext = self.open(&sealed)?;
        String::from_utf8(plaintext).map_err(|_| Error::Crypto)
    }

    /// Decrypt preview text bound to a storage context.
    pub fn open_text_with_aad(&self, aad: &[u8], sealed_hex: &str) -> Result<String> {
        let sealed = hex_decode(sealed_hex).ok_or(Error::Crypto)?;
        let plaintext = self.open_with_aad(aad, &sealed)?;
        String::from_utf8(plaintext).map_err(|_| Error::Crypto)
    }
}

/// 16 hex characters derived from the key under a fixed context string.
fn key_fingerprint(key: &[u8; KEY_LEN]) -> String {
    let derived = blake3::derive_key("panora master key fingerprint v1", key);
    hex_encode(&derived[..8])
}

/// Lowercase hex encoding (avoids pulling in a hex crate).
pub fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0x0f) as usize] as char);
    }
    s
}

/// Hex decoding counterpart of `hex_encode`; None on malformed input.
pub fn hex_decode(s: &str) -> Option<Vec<u8>> {
    let bytes = s.as_bytes();
    if !bytes.len().is_multiple_of(2) {
        return None;
    }
    let val = |c: u8| -> Option<u8> {
        match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'a'..=b'f' => Some(c - b'a' + 10),
            b'A'..=b'F' => Some(c - b'A' + 10),
            _ => None,
        }
    };
    let mut out = Vec::with_capacity(bytes.len() / 2);
    for pair in bytes.as_chunks::<2>().0 {
        out.push((val(pair[0])? << 4) | val(pair[1])?);
    }
    Some(out)
}

/// BLAKE3 content hash, hex-encoded. Used for dedup and blob naming.
pub fn content_hash(data: &[u8]) -> String {
    blake3::hash(data).to_hex().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seal_open_roundtrip() {
        let key = MasterKey::generate();
        let cipher = Cipher::new(&key);
        let msg = b"gizli pano icerigi \xF0\x9F\x94\x92";
        let sealed = cipher.seal(msg).unwrap();
        assert!(sealed.starts_with(ENVELOPE_MAGIC));
        assert_eq!(sealed[ENVELOPE_MAGIC.len()], ENVELOPE_VERSION);
        assert_ne!(&sealed[ENVELOPE_HEADER_LEN + NONCE_LEN..], msg);
        let opened = cipher.open(&sealed).unwrap();
        assert_eq!(opened, msg);
    }

    #[test]
    fn aad_binds_storage_context() {
        let key = MasterKey::generate();
        let cipher = Cipher::new(&key);
        let sealed = cipher.seal_with_aad(b"blob/hash", b"secret").unwrap();
        assert_eq!(
            cipher.open_with_aad(b"blob/hash", &sealed).unwrap(),
            b"secret"
        );
        assert!(cipher.open_with_aad(b"other/hash", &sealed).is_err());
        assert!(cipher.open(&sealed).is_err());
    }

    #[test]
    fn nonces_are_unique() {
        let key = MasterKey::generate();
        let cipher = Cipher::new(&key);
        let a = cipher.seal(b"same").unwrap();
        let b = cipher.seal(b"same").unwrap();
        assert_ne!(a, b, "two seals of identical plaintext must differ");
    }

    #[test]
    fn wrong_key_fails() {
        let c1 = Cipher::new(&MasterKey::generate());
        let c2 = Cipher::new(&MasterKey::generate());
        let sealed = c1.seal(b"data").unwrap();
        assert!(c2.open(&sealed).is_err());
    }

    #[test]
    fn tampered_ciphertext_fails() {
        let key = MasterKey::generate();
        let cipher = Cipher::new(&key);
        let mut sealed = cipher.seal(b"integrity matters").unwrap();
        let last = sealed.len() - 1;
        sealed[last] ^= 0x01;
        assert!(cipher.open(&sealed).is_err());
    }

    #[test]
    fn truncated_input_fails() {
        let key = MasterKey::generate();
        let cipher = Cipher::new(&key);
        assert!(cipher.open(&[0u8; 8]).is_err());
        assert!(cipher
            .open(
                &[
                    ENVELOPE_MAGIC.as_slice(),
                    &[ENVELOPE_VERSION],
                    &[0u8; NONCE_LEN + TAG_LEN - 1],
                ]
                .concat()
            )
            .is_err());
    }

    #[test]
    fn text_roundtrip() {
        let key = MasterKey::generate();
        let cipher = Cipher::new(&key);
        let sealed = cipher.seal_text("merhaba dünya").unwrap();
        assert_eq!(cipher.open_text(&sealed).unwrap(), "merhaba dünya");
        assert!(cipher.open_text("not-hex!").is_err());
    }

    #[test]
    fn text_aad_binds_preview_context() {
        let key = MasterKey::generate();
        let cipher = Cipher::new(&key);
        let sealed = cipher
            .seal_text_with_aad(b"entry-preview/42", "preview")
            .unwrap();
        assert_eq!(
            cipher
                .open_text_with_aad(b"entry-preview/42", &sealed)
                .unwrap(),
            "preview"
        );
        assert!(cipher
            .open_text_with_aad(b"entry-preview/43", &sealed)
            .is_err());
    }

    #[test]
    fn hex_roundtrip() {
        let bytes = [0x00, 0xab, 0xff, 0x10];
        assert_eq!(hex_encode(&bytes), "00abff10");
        assert_eq!(hex_decode("00abff10").unwrap(), bytes);
        assert!(hex_decode("abc").is_none());
        assert!(hex_decode("zz").is_none());
    }

    #[test]
    fn content_hash_is_stable() {
        assert_eq!(content_hash(b"panora"), content_hash(b"panora"));
        assert_ne!(content_hash(b"a"), content_hash(b"b"));
        assert_eq!(content_hash(b"").len(), 64);
    }

    #[test]
    fn master_key_debug_is_redacted() {
        let key = MasterKey::generate();
        let dbg = format!("{key:?}");
        assert!(dbg.contains("REDACTED"));
        assert!(!dbg.contains(&hex_encode(key.as_bytes())));
    }
}
