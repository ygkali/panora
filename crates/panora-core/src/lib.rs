// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only
//
// Panora core library: storage, encryption, privacy engine, backend abstraction.
//
// This crate is display-server agnostic. Clipboard capture lives behind the
// `ClipboardBackend` trait; concrete backends (X11, Wayland, GNOME bridge)
// live in the daemon crate.

//! Shared, encrypted clipboard data model and storage primitives for Panora.
//!
//! This crate is display-server agnostic. Concrete X11, Wayland and GNOME
//! capture implementations live in the daemon crate.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

/// Application classes with special needs (terminals paste with Ctrl+Shift+V).
pub mod apps;
/// Clipboard backend abstraction (trait + event types).
pub mod backend;
/// Encrypted export/import archive format (CLI-03).
pub mod backup;
/// Code-detection heuristic for the popup's monospace font (UI-10).
pub mod code;
/// Configuration loading and defaults.
pub mod config;
/// Error types shared across the crate.
pub mod error;
/// User-facing strings in Turkish and English.
pub mod i18n;
/// JSON-lines IPC protocol shared by daemon, GUI and CLI.
pub mod ipc;
/// Second-layer password lock (SEC-02), on top of the always-loaded master key.
pub mod lock;
/// Data model (entries, MIME payloads, content kinds).
pub mod model;
/// Privacy engine: secret flags, exclusion lists, private mode.
pub mod privacy;
/// HTML to Pango markup for the rich-text preview (UI-22).
pub mod richtext;
/// The search grammar (`kind:` `app:` `after:` `re:` ...) and match highlighting.
pub mod search;
/// Heuristics for secrets, keys and card numbers (`privacy.sensitive_policy`).
pub mod sensitive;
/// Encrypted storage: SQLite + FTS5 index + content-addressed blob store.
pub mod storage;
/// Sync extension point (ADR 0002): trait + no-op stub. Real sync ships later.
pub mod sync;

pub use error::{Error, Result};
