// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

#![no_main]
//! SEC-06: content classification (`ClipboardData::classify`/`text`), the
//! first thing every captured clipboard change goes through — the mime
//! list and the payload bytes both come straight from whatever
//! application owns the selection, not from Panora.

use libfuzzer_sys::fuzz_target;
use panora_core::model::{ClipboardData, MimePayload, Selection};

// A representative spread of MIME types real applications offer; the
// control byte in `data` just picks which shape this run exercises,
// leaving the rest of `data` as the payload bytes.
const MIME_SHAPES: &[&[&str]] = &[
    &["text/plain"],
    &["text/plain", "text/html"],
    &["text/uri-list"],
    &["x-special/gnome-copied-files", "text/uri-list"],
    &["image/png"],
    &["text/x-color"],
    &["application/octet-stream"],
];

fuzz_target!(|data: &[u8]| {
    if data.is_empty() {
        return;
    }
    let shape = MIME_SHAPES[data[0] as usize % MIME_SHAPES.len()];
    let payload_bytes = &data[1..];
    let clipboard_data = ClipboardData {
        selection: Selection::Clipboard,
        offered_mimes: shape.iter().map(|m| m.to_string()).collect(),
        payloads: shape
            .iter()
            .map(|mime| MimePayload::new(*mime, payload_bytes.to_vec()))
            .collect(),
        source_app: None,
    };
    let _ = clipboard_data.classify();
    let _ = clipboard_data.text();
});
