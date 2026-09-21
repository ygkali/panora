// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

#![no_main]
//! SEC-06: turning a search string into an FTS5 `MATCH` expression. A bug
//! here could let a crafted search string break out of the intended query
//! shape, not just crash the process.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(input) = std::str::from_utf8(data) {
        let _ = panora_core::storage::fts_query(input);
    }
});
