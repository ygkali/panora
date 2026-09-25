// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! Long-term device identity: one Ed25519 key pair per device.
//!
//! The public key is what the roster lists and what the other devices pin;
//! SYNC-04 turns the same PKCS#8 document into the device's self-signed
//! QUIC certificate (ADR 0004 §5), so a TLS peer is authenticated by
//! checking its certificate key against the roster.

use crate::error::{Error, Result};
use ring::rand::SystemRandom;
use ring::signature::{Ed25519KeyPair, KeyPair, UnparsedPublicKey, ED25519};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

/// Length of an Ed25519 public key.
pub const PUBLIC_KEY_LEN: usize = 32;
/// Length of an Ed25519 signature.
pub const SIGNATURE_LEN: usize = 64;

/// A device's public identity key, as the roster and the peers see it.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PublicIdentity(#[serde(with = "crate::bytes::b64")] [u8; PUBLIC_KEY_LEN]);

impl PublicIdentity {
    /// Wrap raw public key bytes. Nothing is validated here; a key that is
    /// not a valid curve point simply never verifies a signature.
    pub fn from_bytes(bytes: [u8; PUBLIC_KEY_LEN]) -> Self {
        Self(bytes)
    }

    /// Raw public key bytes.
    pub fn as_bytes(&self) -> &[u8; PUBLIC_KEY_LEN] {
        &self.0
    }

    /// A short, human-comparable form: 80 bits of a BLAKE3 derivation of
    /// the key, as five groups of four hex digits (`3f2a-91c0-…`). Shown in
    /// the device list so a user can tell two devices of the same name
    /// apart; it is not what pairing security rests on.
    pub fn fingerprint(&self) -> String {
        let digest = blake3::derive_key("panora identity fingerprint v1", &self.0);
        digest[..10]
            .chunks(2)
            .map(|pair| format!("{:02x}{:02x}", pair[0], pair[1]))
            .collect::<Vec<_>>()
            .join("-")
    }

    /// Check an Ed25519 signature made by this identity.
    pub fn verify(&self, message: &[u8], signature: &[u8; SIGNATURE_LEN]) -> Result<()> {
        UnparsedPublicKey::new(&ED25519, &self.0)
            .verify(message, signature)
            .map_err(|_| Error::Auth("signature does not match the device identity"))
    }
}

impl std::fmt::Debug for PublicIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PublicIdentity({})", self.fingerprint())
    }
}

/// This device's identity key pair.
pub struct DeviceIdentity {
    pkcs8: Zeroizing<Vec<u8>>,
    pair: Ed25519KeyPair,
    public: PublicIdentity,
}

impl DeviceIdentity {
    /// Generate a new identity from the operating system's CSPRNG.
    pub fn generate() -> Result<Self> {
        let document =
            Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).map_err(|_| Error::Crypto)?;
        Self::from_pkcs8(document.as_ref())
    }

    /// Load an identity from its PKCS#8 (v2) document.
    pub fn from_pkcs8(pkcs8: &[u8]) -> Result<Self> {
        let pair = Ed25519KeyPair::from_pkcs8(pkcs8).map_err(|_| Error::Crypto)?;
        let public: [u8; PUBLIC_KEY_LEN] = pair
            .public_key()
            .as_ref()
            .try_into()
            .map_err(|_| Error::Crypto)?;
        Ok(Self {
            pkcs8: Zeroizing::new(pkcs8.to_vec()),
            pair,
            public: PublicIdentity(public),
        })
    }

    /// The PKCS#8 document, for the encrypted state file and, in SYNC-04,
    /// the TLS certificate. Secret.
    pub fn pkcs8(&self) -> &[u8] {
        &self.pkcs8
    }

    /// The public half.
    pub fn public(&self) -> PublicIdentity {
        self.public
    }

    /// Sign `message`.
    pub fn sign(&self, message: &[u8]) -> [u8; SIGNATURE_LEN] {
        let signature = self.pair.sign(message);
        let mut out = [0u8; SIGNATURE_LEN];
        out.copy_from_slice(signature.as_ref());
        out
    }
}

impl std::fmt::Debug for DeviceIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "DeviceIdentity({}, [REDACTED])",
            self.public.fingerprint()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_verify_and_reload() {
        let id = DeviceIdentity::generate().unwrap();
        let sig = id.sign(b"hello");
        id.public().verify(b"hello", &sig).unwrap();
        assert!(id.public().verify(b"hellO", &sig).is_err());

        let again = DeviceIdentity::from_pkcs8(id.pkcs8()).unwrap();
        assert_eq!(again.public(), id.public());
        again
            .public()
            .verify(b"hello", &again.sign(b"hello"))
            .unwrap();
    }

    #[test]
    fn another_identity_does_not_verify() {
        let a = DeviceIdentity::generate().unwrap();
        let b = DeviceIdentity::generate().unwrap();
        assert_ne!(a.public(), b.public());
        assert!(b.public().verify(b"m", &a.sign(b"m")).is_err());
    }

    #[test]
    fn fingerprint_shape_and_debug_hides_the_key() {
        let id = DeviceIdentity::generate().unwrap();
        let fp = id.public().fingerprint();
        assert_eq!(fp.len(), 24);
        assert_eq!(fp.matches('-').count(), 4);
        assert!(format!("{id:?}").contains("REDACTED"));
        assert!(DeviceIdentity::from_pkcs8(b"not a key").is_err());
    }
}
