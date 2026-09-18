// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! In-process stand-in for the daemon, compiled only with the `fixture`
//! feature (`cargo run -p panora-gui --features fixture`). Lets the popup be
//! exercised on machines without panod: every request is answered from a
//! canned history so cards, previews, dialogs and pagination can be seen.

use panora_core::error::Result;
use panora_core::ipc::{CapabilityData, Request, ResponseData, StatusData, PROTOCOL_VERSION};
use panora_core::model::{ContentKind, Entry, MimePayload, Selection};
use std::sync::Mutex;

/// A small PNG rendered through gdk-pixbuf, enough to exercise the image path.
fn png() -> &'static [u8] {
    static PNG: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
    PNG.get_or_init(|| {
        let pixbuf =
            gdk_pixbuf::Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, false, 8, 96, 64).expect("pixbuf");
        pixbuf.fill(0x3584e4ff);
        pixbuf
            .save_to_bufferv("png", &[])
            .map(|b| b.to_vec())
            .unwrap_or_default()
    })
}

struct Store {
    entries: Vec<Entry>,
    private: bool,
    revision: u64,
}

static STORE: Mutex<Option<Store>> = Mutex::new(None);

fn entry(id: i64, kind: ContentKind, preview: &str, mime: &str, size: i64, age: i64) -> Entry {
    let now = crate::util::unix_now();
    Entry {
        id,
        content_hash: format!("{id:064x}"),
        preview: preview.into(),
        kind,
        primary_mime: mime.into(),
        size_bytes: size,
        source_app: Some(
            match id % 3 {
                0 => "firefox",
                1 => "gnome-text-editor",
                _ => "nautilus",
            }
            .into(),
        ),
        created_at: now - age,
        last_seen_at: now - age,
        pinned: id == 2,
        selection: Selection::Clipboard,
        device_id: "fixture".into(),
        lamport: id,
        deleted: false,
    }
}

fn seed() -> Store {
    let mut entries = vec![
        entry(1, ContentKind::Text, "Merhaba dünya — bu bir düz metin kaydı. Kartlar içeriklerine göre boyutlanır ve dört satırdan sonra kısaltılır; ayrıntı görünümünde tamamı okunur.", "text/plain;charset=utf-8", 148, 20),
        entry(2, ContentKind::Link, "https://gitlab.gnome.org/GNOME/mutter/-/merge_requests", "text/plain", 54, 400),
        entry(3, ContentKind::Image, "[image]", "image/png", png().len() as i64, 3_700),
        entry(4, ContentKind::Color, "#3584e4", "text/plain", 7, 90_000),
        entry(5, ContentKind::RichText, "Hello world again", "text/html", 61, 700_000),
        entry(6, ContentKind::FileList, "rapor son.pdf\nx.png", "text/uri-list", 72, 30),
    ];
    for i in 7..=80 {
        entries.push(entry(
            i,
            ContentKind::Text,
            &format!("Sayfalama için doldurma kaydı {i}"),
            "text/plain",
            32,
            i * 60,
        ));
    }
    Store {
        entries,
        private: false,
        revision: 1,
    }
}

fn with_store<T>(f: impl FnOnce(&mut Store) -> T) -> T {
    let mut guard = STORE.lock().unwrap_or_else(|e| e.into_inner());
    let store = guard.get_or_insert_with(seed);
    f(store)
}

fn payloads_for(entry: &Entry) -> Vec<MimePayload> {
    match entry.kind {
        ContentKind::Image => vec![MimePayload::new("image/png", png())],
        ContentKind::RichText => vec![
            MimePayload::new("text/plain;charset=utf-8", "Hello world again"),
            MimePayload::new("text/html", "<p>Hello <b>world</b></p><p>again</p>"),
        ],
        ContentKind::FileList => vec![MimePayload::new(
            "text/uri-list",
            "file:///home/u/Belgeler/rapor%20son.pdf\r\nfile:///tmp/x.png\r\n",
        )],
        _ => vec![MimePayload::new(
            entry.primary_mime.as_str(),
            entry.preview.as_bytes(),
        )],
    }
}

/// Answer a request from the canned history.
pub fn call(request: &Request) -> Result<ResponseData> {
    with_store(|store| {
        Ok(match request {
            Request::List(q) => {
                let needle = q.search.as_deref().map(str::to_lowercase);
                let entries: Vec<Entry> = store
                    .entries
                    .iter()
                    .filter(|e| q.kind.as_deref().is_none_or(|k| e.kind.as_str() == k))
                    .filter(|e| !q.pinned_only || e.pinned)
                    .filter(|e| {
                        needle
                            .as_deref()
                            .is_none_or(|n| e.preview.to_lowercase().contains(n))
                    })
                    .skip(q.offset)
                    .take(if q.limit == 0 { 50 } else { q.limit })
                    .cloned()
                    .collect();
                ResponseData::Entries(entries)
            }
            Request::Recall { paste, .. } => ResponseData::Recalled { pasted: !paste },
            Request::Pin { id, pinned } => {
                if let Some(e) = store.entries.iter_mut().find(|e| e.id == *id) {
                    e.pinned = *pinned;
                }
                store.revision += 1;
                ResponseData::Empty
            }
            Request::Delete { id } => {
                store.entries.retain(|e| e.id != *id);
                store.revision += 1;
                ResponseData::Empty
            }
            Request::Clear => {
                let before = store.entries.len();
                store.entries.retain(|e| e.pinned);
                store.revision += 1;
                ResponseData::Count(before - store.entries.len())
            }
            Request::SetPrivate { enabled } => {
                store.private = *enabled;
                ResponseData::Empty
            }
            Request::Toggle | Request::ReloadConfig => ResponseData::Empty,
            Request::Status => ResponseData::Status(StatusData {
                backend: "fixture".into(),
                entries: store.entries.len() as i64,
                private_mode: store.private,
                sync_active: false,
                revision: store.revision,
                version: env!("CARGO_PKG_VERSION").into(),
                protocol: PROTOCOL_VERSION,
                capabilities: CapabilityData {
                    primary: true,
                    images: true,
                    persist: true,
                    synthetic_paste: false,
                    source_app: true,
                    needs_bridge: false,
                },
            }),
            Request::Preview { id } => ResponseData::Payloads(
                store
                    .entries
                    .iter()
                    .find(|e| e.id == *id)
                    .map(payloads_for)
                    .unwrap_or_default(),
            ),
        })
    })
}
