// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! Panora daemon library: backends, capture pipeline, GNOME bridge, keyring
//! and the Unix-socket IPC server.

#![forbid(unsafe_code)]

pub mod backend;
pub mod daemon;
pub mod gnome;
#[cfg(unix)]
pub mod keyring;
pub mod paste;
#[cfg(unix)]
pub mod server;

/// IPC protocol types, shared with the clients through `panora-core`.
pub use panora_core::ipc;
