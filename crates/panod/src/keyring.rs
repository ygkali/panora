// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! Master key management via the freedesktop Secret Service API.
//!
//! The session is opened with the `dh-ietf1024-sha256-aes128-cbc-pkcs7`
//! algorithm, so the master key is encrypted on the session bus rather than
//! travelling in the clear. That matters because a sandboxed application
//! holding only `--socket=session-bus` can observe bus traffic without any
//! access to Panora's data directory.
//!
//! The key never touches disk in plaintext. A missing item in an existing,
//! unlocked collection is the only condition that creates a new key. Locked
//! collections, missing services, a service that cannot negotiate an
//! encrypted session, and dismissed prompts all fail closed instead of
//! silently generating a replacement key.

use std::collections::HashMap;

use oo7::dbus::{Collection, Service};
use oo7::Secret;
use panora_core::error::{Error, Result};
use panora_core::storage::MasterKey;
use tracing::info;

/// How long the whole Secret Service bring-up may take, prompts included.
const KEYRING_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

const COLLECTION_ALIAS: &str = "default";
const ITEM_LABEL: &str = "Panora master key";
const ITEM_ATTRS: &[(&str, &str)] = &[
    ("application", "panora"),
    ("purpose", "clipboard-history-encryption"),
    ("format_version", "1"),
];

/// The key a `rotate-key` in progress is resealing everything under, kept
/// separate from the live item so a crash mid-rotation still has somewhere
/// durable to resume from (SEC-01): the live item is only replaced, and
/// this one only deleted, once every stored ciphertext is already resealed.
const PENDING_ITEM_LABEL: &str = "Panora master key (rotation pending)";
const PENDING_ITEM_ATTRS: &[(&str, &str)] = &[
    ("application", "panora"),
    ("purpose", "clipboard-history-encryption"),
    ("format_version", "1"),
    ("role", "pending"),
];

fn attributes() -> HashMap<&'static str, &'static str> {
    ITEM_ATTRS.iter().copied().collect()
}

fn pending_attributes() -> HashMap<&'static str, &'static str> {
    PENDING_ITEM_ATTRS.iter().copied().collect()
}

fn keyring_err(context: &str, e: impl std::fmt::Display) -> Error {
    Error::Keyring(format!("{context}: {e}"))
}

/// Load the master key from Secret Service, or create it on first use.
///
/// Bounded as a whole: creating or unlocking a collection raises a prompt on
/// the service, and a session with no prompter never answers it. Left alone the
/// daemon sits there "running" with no socket and no log line -- and oo7 itself
/// eventually panics on the abandoned prompt. One timeout over the entire
/// bring-up keeps every one of those paths from hanging panod.
pub async fn load_or_create_master_key() -> Result<MasterKey> {
    match tokio::time::timeout(KEYRING_TIMEOUT, bring_up_key()).await {
        Ok(result) => result,
        Err(_) => Err(Error::Keyring(format!(
            "Secret Service did not answer within {}s; an unlock prompt may be \
             waiting. Unlock your login keyring, then: \
             systemctl --user reset-failed panod.service && \
             systemctl --user restart panod.service",
            KEYRING_TIMEOUT.as_secs()
        ))),
    }
}

async fn bring_up_key() -> Result<MasterKey> {
    let collection = open_collection().await?;
    match load_key(&collection).await? {
        Some(key) => {
            info!("master key loaded from Secret Service");
            Ok(key)
        }
        None => {
            info!("Panora keyring item not found; generating a new master key");
            let key = MasterKey::generate();
            store_key(&collection, &key).await?;
            Ok(key)
        }
    }
}

/// True when the service is telling us the object simply is not there.
///
/// A `default` alias can outlive the collection it points at; the service then
/// answers method calls on that path with `UnknownMethod`/`UnknownObject`
/// rather than reporting the alias as unset. Narrow on purpose: any other
/// failure must stay an error, because treating it as "nothing here" would
/// create a second collection and orphan an existing key.
fn is_missing_object(e: &oo7::dbus::Error) -> bool {
    use oo7::dbus::{Error as DbusError, ServiceError};
    match e {
        DbusError::Deleted | DbusError::NotFound(_) => true,
        DbusError::Service(ServiceError::NoSuchObject(_)) => true,
        DbusError::ZBus(zbus::Error::MethodError(name, ..)) => matches!(
            name.as_str(),
            "org.freedesktop.DBus.Error.UnknownMethod"
                | "org.freedesktop.DBus.Error.UnknownObject"
                | "org.freedesktop.DBus.Error.UnknownInterface"
        ),
        _ => false,
    }
}

fn locked() -> Error {
    Error::Keyring("keyring collection is locked; unlock it and retry".into())
}

/// Unlock a collection that reported itself locked, and confirm it opened.
///
/// A collection created on the fly comes back locked, and a login keyring is
/// locked whenever `pam_gnome_keyring` did not run at login. Asking the service
/// to unlock is the normal path -- it prompts the user when it needs to. This
/// stays fail-closed: we only proceed once the collection reports itself
/// unlocked, so a locked collection is never mistaken for an empty one.
async fn unlock(collection: &Collection) -> Result<()> {
    info!("keyring collection is locked; requesting unlock (a prompt may appear)");
    collection
        .unlock(None)
        .await
        .map_err(|e| keyring_err("keyring unlock failed", e))?;

    if collection
        .is_locked()
        .await
        .map_err(|e| keyring_err("cannot read the collection lock state", e))?
    {
        return Err(locked());
    }
    Ok(())
}

/// Open the default collection over an encrypted Secret Service session.
///
/// `Service::encrypted` is deliberate: `Service::new` falls back to the plain
/// algorithm when the DH handshake fails, which would put the master key back
/// on the bus unencrypted. Refusing is the safer failure here.
async fn open_collection() -> Result<Collection> {
    let service = Service::encrypted()
        .await
        .map_err(|e| keyring_err("cannot open an encrypted Secret Service session", e))?;

    // Three outcomes: a usable collection, nothing at all, or a hard error.
    // The alias can outlive the collection it points at, so a "no such object"
    // answer here means "nothing there" -- anything else stays fatal.
    let existing = match service.with_alias(COLLECTION_ALIAS).await {
        Ok(Some(collection)) => match collection.is_locked().await {
            Ok(is_locked) => Some((collection, is_locked)),
            Err(e) if is_missing_object(&e) => None,
            Err(e) => return Err(keyring_err("cannot read the collection lock state", e)),
        },
        Ok(None) => None,
        Err(e) if is_missing_object(&e) => None,
        Err(e) => {
            return Err(keyring_err(
                "cannot reach the default keyring collection",
                e,
            ))
        }
    };

    if let Some((collection, is_locked)) = existing {
        if is_locked {
            unlock(&collection).await?;
        }
        return Ok(collection);
    }

    // Nothing to lose here: with no collection there is no stored key that a
    // freshly created one could orphan. Machines that never had a login keyring
    // -- minimal installs, or a session where pam_gnome_keyring never ran --
    // land here instead of failing to start.
    info!("no usable default keyring collection; creating one");
    let collection = service
        .default_collection()
        .await
        .map_err(|e| keyring_err("cannot create a keyring collection", e))?;
    // A freshly created collection comes back locked.
    if collection
        .is_locked()
        .await
        .map_err(|e| keyring_err("cannot read the collection lock state", e))?
    {
        unlock(&collection).await?;
    }
    Ok(collection)
}

/// Read the key from the collection if the item is present.
async fn load_key(collection: &Collection) -> Result<Option<MasterKey>> {
    load_key_with(collection, &attributes()).await
}

/// The pending rotation key (SEC-01), if a `rotate-key` is in progress.
pub async fn load_pending_key() -> Result<Option<MasterKey>> {
    let collection = open_collection().await?;
    load_key_with(&collection, &pending_attributes()).await
}

async fn load_key_with(
    collection: &Collection,
    attrs: &HashMap<&'static str, &'static str>,
) -> Result<Option<MasterKey>> {
    let items = collection
        .search_items(attrs)
        .await
        .map_err(|e| keyring_err("keyring search failed", e))?;

    let Some(item) = items.first() else {
        return Ok(None);
    };

    if item
        .is_locked()
        .await
        .map_err(|e| keyring_err("cannot read the item lock state", e))?
    {
        return Err(Error::Keyring(
            "Panora keyring item is locked; unlock the collection and retry".into(),
        ));
    }

    let secret = item
        .secret()
        .await
        .map_err(|e| keyring_err("cannot read the stored master key", e))?;

    let bytes: [u8; 32] = secret
        .as_bytes()
        .try_into()
        .map_err(|_| Error::Keyring("stored master key has wrong length".into()))?;
    Ok(Some(MasterKey::from_bytes(bytes)))
}

/// Store the key a `rotate-key` (SEC-01) is resealing everything under,
/// durably, before any stored ciphertext is touched — a crash after this
/// still has somewhere to resume the same rotation from instead of losing
/// track of which key half the data ends up under.
pub async fn store_pending_key(key: &MasterKey) -> Result<()> {
    let collection = open_collection().await?;
    collection
        .create_item(
            PENDING_ITEM_LABEL,
            &pending_attributes(),
            Secret::blob(key.as_bytes()),
            true,
            None,
        )
        .await
        .map_err(|e| keyring_err("cannot store the pending rotation key", e))?;
    Ok(())
}

/// Finish a rotation: replace the live master key item with `key` and
/// remove the pending one. Only called once `Database::rekey` and
/// `BlobStore::rekey` have both already succeeded under `key`, so nothing
/// stored ever depends on an item this deletes.
pub async fn finish_rotation(key: &MasterKey) -> Result<()> {
    let collection = open_collection().await?;
    store_key(&collection, key).await?;
    let pending = collection
        .search_items(&pending_attributes())
        .await
        .map_err(|e| keyring_err("keyring search failed", e))?;
    if let Some(item) = pending.first() {
        item.delete(None)
            .await
            .map_err(|e| keyring_err("cannot remove the pending rotation key", e))?;
    }
    Ok(())
}

/// Store a freshly generated key in the unlocked collection.
async fn store_key(collection: &Collection, key: &MasterKey) -> Result<()> {
    collection
        .create_item(
            ITEM_LABEL,
            &attributes(),
            Secret::blob(key.as_bytes()),
            true,
            None,
        )
        .await
        .map_err(|e| keyring_err("cannot store the master key", e))?;
    Ok(())
}
