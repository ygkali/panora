// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

#![no_main]
//! SEC-06: `Request`/`Response` JSON-lines decoding (v2). Every byte a
//! client sends over the IPC socket reaches `decode` before anything else
//! looks at it; it must never panic no matter what garbage arrives.

use libfuzzer_sys::fuzz_target;
use panora_core::ipc::{decode, Request};

fuzz_target!(|data: &[u8]| {
    let _ = decode::<Request>(data);
});
