// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

#![no_main]
//! SEC-06: the v3 frame header (STO-08) — `panora_core::ipc::v3::
//! fuzz_parse_request_header` runs `data` through the exact `serde_json::
//! from_slice::<FrameHeaderIn<Request>>` call a real connection's header
//! bytes go through, before anything else about the frame is trusted.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    panora_core::ipc::v3::fuzz_parse_request_header(data);
});
