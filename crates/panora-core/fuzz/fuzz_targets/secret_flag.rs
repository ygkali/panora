// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

#![no_main]
//! SEC-06: the secret/concealed-type MIME flag check
//! (`PrivacyEngine::has_secret_flag`, ADR 0003) — the TARGETS list an
//! arbitrary application offers on the clipboard, checked before any
//! payload is ever read. Splits `data` on newlines into candidate MIME
//! strings, the same shape the real offered-MIME list has.

use libfuzzer_sys::fuzz_target;
use panora_core::privacy::PrivacyEngine;

fuzz_target!(|data: &[u8]| {
    if let Ok(text) = std::str::from_utf8(data) {
        let mimes: Vec<String> = text.lines().map(str::to_string).collect();
        let _ = PrivacyEngine::has_secret_flag(&mimes);
    }
});
