// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

#![no_main]
//! SEC-06: the search query grammar (`kind:`/`app:`/`pinned:`/`before:`/
//! `after:`/`re:`, quoted phrases). `panora-cli search`/`list` hand
//! whatever the caller typed straight to this before it ever reaches SQL.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(input) = std::str::from_utf8(data) {
        let _ = panora_core::search::parse(input, 1_700_000_000);
    }
});
