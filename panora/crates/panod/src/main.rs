// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! panod executable: starts the display backend, encrypted store and
//! versioned Unix-socket IPC service.

#![forbid(unsafe_code)]

#[cfg(unix)]
fn main() -> anyhow::Result<()> {
    panod::server::main()
}

#[cfg(not(unix))]
fn main() {
    eprintln!("panod runs on Linux desktops only (X11 or Wayland)");
    std::process::exit(1);
}
