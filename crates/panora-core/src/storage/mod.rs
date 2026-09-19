// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! Encrypted SQLite metadata and content-addressed BLOB storage.

pub mod blob;
pub mod crypto;
pub mod db;

pub use blob::BlobStore;
pub use crypto::{content_hash, Cipher, MasterKey};
pub use db::{fts_query, Database, QueryFilter};
