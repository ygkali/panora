// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

#![no_main]
//! SEC-06: opening the versioned storage envelope (`crypto.rs`'s `PNR1`
//! magic + version + nonce + XChaCha20-Poly1305 ciphertext, plus the
//! pre-`PNR1` legacy form `open`/`open_with_aad` still accept). Every
//! preview and blob on disk passes through this on every read; a crash
//! here is a crash on a corrupt or truncated file, not just bad input over
//! the network.

use libfuzzer_sys::fuzz_target;
use panora_core::storage::{Cipher, MasterKey};

fuzz_target!(|data: &[u8]| {
    // A fixed key: what matters is that parsing an arbitrary envelope
    // never panics, not whether it happens to decrypt (it won't).
    let cipher = Cipher::new(&MasterKey::from_bytes([0x42; 32]));
    let split = data.len() / 2;
    let (aad, sealed) = data.split_at(split);
    let _ = cipher.open(sealed);
    let _ = cipher.open_with_aad(aad, sealed);
});
