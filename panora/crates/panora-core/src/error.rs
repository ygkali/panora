// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! Shared error types for panora-core.

use thiserror::Error;

/// Unified result alias for the crate.
pub type Result<T> = std::result::Result<T, Error>;

/// All recoverable errors produced by panora-core.
#[derive(Debug, Error)]
pub enum Error {
    /// SQLite layer failure.
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),

    /// Filesystem failure (blob store, config, sockets).
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// Encryption or decryption failure. Deliberately carries no detail
    /// so that ciphertext/oracle information never leaks through logs.
    #[error("cryptographic operation failed")]
    Crypto,

    /// Keyring (Secret Service) unavailable or entry missing.
    #[error("secret service error: {0}")]
    Keyring(String),

    /// Configuration parse or validation failure.
    #[error("config error: {0}")]
    Config(String),

    /// Clipboard backend failure (backend-specific message).
    #[error("clipboard backend error: {0}")]
    Backend(String),

    /// Requested entry does not exist.
    #[error("entry not found: {0}")]
    NotFound(i64),

    /// Payload exceeds the configured per-MIME size limit.
    #[error("payload too large: {size} bytes (limit {limit})")]
    TooLarge {
        /// Actual payload size in bytes.
        size: usize,
        /// Configured limit in bytes.
        limit: usize,
    },

    /// Content was rejected by the privacy engine (secret flag, excluded
    /// app, or private mode). This is not a failure of the caller; it is
    /// the privacy engine doing its job.
    #[error("content rejected by privacy policy")]
    PrivacyRejected,

    /// Serialization failure.
    #[error("serialization error: {0}")]
    Serialization(String),
}

impl From<chacha20poly1305::Error> for Error {
    fn from(_: chacha20poly1305::Error) -> Self {
        Error::Crypto
    }
}

impl From<toml::de::Error> for Error {
    fn from(e: toml::de::Error) -> Self {
        Error::Config(e.to_string())
    }
}

impl From<toml::ser::Error> for Error {
    fn from(e: toml::ser::Error) -> Self {
        Error::Config(e.to_string())
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error::Serialization(e.to_string())
    }
}
