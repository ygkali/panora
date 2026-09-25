// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! The key that seals the sync state file, kept in the Secret Service like
//! `panod`'s master key (same encrypted session, same fail-closed rules: a
//! locked or unreachable keyring is an error, never a reason to make a new
//! key and orphan the old state).

use crate::error::{Error, Result};
use oo7::dbus::{Collection, Service};
use oo7::Secret;
use panora_core::storage::MasterKey;
use std::collections::HashMap;
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(60);
const ITEM_LABEL: &str = "Panora sync state key";
const ITEM_ATTRS: &[(&str, &str)] = &[
    ("application", "panora"),
    ("purpose", "sync-state-encryption"),
    ("format_version", "1"),
    ("role", "primary"),
];

fn err(context: &str, e: impl std::fmt::Display) -> Error {
    Error::Panod(format!("keyring: {context}: {e}"))
}

fn attributes() -> HashMap<&'static str, &'static str> {
    ITEM_ATTRS.iter().copied().collect()
}

/// Load the state key, or create it the first time.
pub async fn load_or_create_state_key() -> Result<MasterKey> {
    tokio::time::timeout(TIMEOUT, bring_up())
        .await
        .map_err(|_| {
            Error::Panod(
                "keyring: the Secret Service did not answer within 60 s; an unlock prompt \
                 may be waiting"
                    .into(),
            )
        })?
}

async fn open_collection() -> Result<Collection> {
    // `encrypted`, not `new`: the key must not cross the session bus in
    // the clear if the DH handshake fails.
    let service = Service::encrypted()
        .await
        .map_err(|e| err("cannot open an encrypted Secret Service session", e))?;
    let collection = match service.with_alias("default").await {
        Ok(Some(collection)) => collection,
        _ => service
            .default_collection()
            .await
            .map_err(|e| err("cannot open the default collection", e))?,
    };
    if collection
        .is_locked()
        .await
        .map_err(|e| err("cannot read the lock state", e))?
    {
        collection
            .unlock(None)
            .await
            .map_err(|e| err("unlock failed", e))?;
        if collection
            .is_locked()
            .await
            .map_err(|e| err("cannot read the lock state", e))?
        {
            return Err(err("collection", "still locked"));
        }
    }
    Ok(collection)
}

async fn bring_up() -> Result<MasterKey> {
    let collection = open_collection().await?;
    let items = collection
        .search_items(&attributes())
        .await
        .map_err(|e| err("search failed", e))?;
    if let Some(item) = items.first() {
        let secret = item
            .secret()
            .await
            .map_err(|e| err("cannot read the state key", e))?;
        let bytes: [u8; 32] = secret
            .as_bytes()
            .try_into()
            .map_err(|_| err("state key", "wrong length"))?;
        return Ok(MasterKey::from_bytes(bytes));
    }
    let key = MasterKey::generate();
    collection
        .create_item(
            ITEM_LABEL,
            &attributes(),
            Secret::blob(key.as_bytes()),
            true,
            None,
        )
        .await
        .map_err(|e| err("cannot store the state key", e))?;
    Ok(key)
}
