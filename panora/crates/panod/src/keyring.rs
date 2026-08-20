// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! Master key management via the freedesktop Secret Service API.
//!
//! The key never touches disk in plaintext. A missing item in an existing,
//! unlocked collection is the only condition that creates a new key. Locked
//! collections, missing services, malformed D-Bus replies and required
//! prompts fail closed instead of silently generating a replacement key.

use panora_core::error::{Error, Result};
use panora_core::storage::MasterKey;
use tracing::info;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, Value};
use zbus::Connection;

const SERVICE: &str = "org.freedesktop.secrets";
const ROOT: &str = "/org/freedesktop/secrets";
const COLLECTION: &str = "/org/freedesktop/secrets/collection/login";
const ITEM_LABEL: &str = "Panora master key";
const ITEM_ATTRS: &[(&str, &str)] = &[
    ("application", "panora"),
    ("purpose", "clipboard-history-encryption"),
    ("format_version", "1"),
];

/// Load the master key from Secret Service, or create it on first use.
pub async fn load_or_create_master_key() -> Result<MasterKey> {
    match load_key().await? {
        Some(key) => {
            info!("master key loaded from Secret Service");
            Ok(key)
        }
        None => {
            info!("Panora keyring item not found; generating a new master key");
            let key = MasterKey::generate();
            store_key(&key).await?;
            Ok(key)
        }
    }
}

/// Open an unlocked Secret Service session using the plain algorithm.
async fn open_session() -> Result<(Connection, OwnedObjectPath)> {
    let conn = Connection::session()
        .await
        .map_err(|e| Error::Keyring(format!("session bus unavailable: {e}")))?;
    let message = conn
        .call_method(
            Some(SERVICE),
            ROOT,
            Some("org.freedesktop.Secret.Service"),
            "OpenSession",
            &("plain", Value::from("")),
        )
        .await
        .map_err(|e| Error::Keyring(format!("OpenSession failed: {e}")))?;
    let body = message.body();
    let reply: (Value<'_>, OwnedObjectPath) = body
        .deserialize()
        .map_err(|e| Error::Keyring(format!("OpenSession reply invalid: {e}")))?;
    Ok((conn, reply.1))
}

/// Close a Secret Service session without masking the primary operation error.
async fn close_session(conn: &Connection, session: &OwnedObjectPath) {
    let _ = conn
        .call_method(
            Some(SERVICE),
            session.as_str(),
            Some("org.freedesktop.Secret.Session"),
            "Close",
            &(),
        )
        .await;
}

/// Search the login collection and read the key if present.
async fn load_key() -> Result<Option<MasterKey>> {
    let (conn, session) = open_session().await?;
    let attrs: std::collections::HashMap<&str, &str> = ITEM_ATTRS.iter().copied().collect();
    let search_message = conn
        .call_method(
            Some(SERVICE),
            COLLECTION,
            Some("org.freedesktop.Secret.Collection"),
            "SearchItems",
            &(attrs),
        )
        .await
        .map_err(|e| Error::Keyring(format!("SearchItems failed: {e}")))?;
    // Secret Service 0.2 specifies (unlocked, locked), while older GNOME
    // Keyring implementations have returned only the unlocked array. Accept
    // both signatures; a locked array is still handled fail-closed when it is
    // available.
    let body = search_message.body();
    let search: (Vec<OwnedObjectPath>, Vec<OwnedObjectPath>) =
        match body.deserialize::<(Vec<OwnedObjectPath>, Vec<OwnedObjectPath>)>() {
            Ok(pair) => pair,
            Err(_) => (
                body.deserialize::<Vec<OwnedObjectPath>>()
                    .map_err(|e| Error::Keyring(format!("SearchItems reply invalid: {e}")))?,
                Vec::new(),
            ),
        };

    if !search.1.is_empty() {
        close_session(&conn, &session).await;
        return Err(Error::Keyring(
            "Panora keyring item is locked; unlock the login collection and retry".into(),
        ));
    }
    let Some(item) = search.0.first() else {
        close_session(&conn, &session).await;
        return Ok(None);
    };

    let secret_result: Result<(OwnedObjectPath, Vec<u8>, Vec<u8>, String)> = conn
        .call_method(
            Some(SERVICE),
            item.as_str(),
            Some("org.freedesktop.Secret.Item"),
            "GetSecret",
            &(&session,),
        )
        .await
        .map_err(|e| Error::Keyring(format!("GetSecret failed: {e}")))?
        .body()
        .deserialize()
        .map_err(|e| Error::Keyring(format!("GetSecret reply invalid: {e}")));
    close_session(&conn, &session).await;
    let secret = secret_result?;
    let bytes: [u8; 32] = secret
        .2
        .try_into()
        .map_err(|_| Error::Keyring("stored master key has wrong length".into()))?;
    Ok(Some(MasterKey::from_bytes(bytes)))
}

/// Store a freshly generated key in the existing unlocked login collection.
async fn store_key(key: &MasterKey) -> Result<()> {
    let (conn, session) = open_session().await?;
    let attrs: std::collections::HashMap<&str, &str> = ITEM_ATTRS.iter().copied().collect();
    let mut props: std::collections::HashMap<&str, Value<'_>> = std::collections::HashMap::new();
    props.insert("org.freedesktop.Secret.Item.Label", Value::from(ITEM_LABEL));
    props.insert(
        "org.freedesktop.Secret.Item.Attributes",
        Value::new(
            attrs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect::<std::collections::HashMap<String, String>>(),
        ),
    );

    let secret: (ObjectPath<'_>, Vec<u8>, Vec<u8>, &str) = (
        (&session).into(),
        Vec::<u8>::new(),
        key.as_bytes().to_vec(),
        "application/octet-stream",
    );
    let result: Result<(OwnedObjectPath, OwnedObjectPath)> = conn
        .call_method(
            Some(SERVICE),
            COLLECTION,
            Some("org.freedesktop.Secret.Collection"),
            "CreateItem",
            &(props, secret, true),
        )
        .await
        .map_err(|e| Error::Keyring(format!("CreateItem failed: {e}")))?
        .body()
        .deserialize()
        .map_err(|e| Error::Keyring(format!("CreateItem reply invalid: {e}")));
    close_session(&conn, &session).await;
    let (_item, prompt) = result?;
    if prompt.as_str() != "/" {
        return Err(Error::Keyring(
            "Secret Service requires an interactive prompt to store the key".into(),
        ));
    }
    Ok(())
}
