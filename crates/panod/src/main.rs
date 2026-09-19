// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! panod executable: starts the display backend, encrypted store and
//! versioned Unix-socket IPC service.

#![forbid(unsafe_code)]

#[cfg(unix)]
fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("--version") | Some("-V") => {
            println!("panod {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        Some("--help") | Some("-h") => {
            println!(
                "panod {}\nPanora clipboard daemon.\n\n\
                 Usage: panod [--version]\n\n\
                 Runs in the foreground; the package installs it as the\n\
                 `panod.service` systemd user unit. Logging follows RUST_LOG\n\
                 (default: info). Configuration: ~/.config/panora/config.toml",
                env!("CARGO_PKG_VERSION")
            );
            return Ok(());
        }
        Some(other) => anyhow::bail!("unknown argument: {other} (try --help)"),
        None => {}
    }
    panod::server::main()
}

#[cfg(not(unix))]
fn main() {
    eprintln!("panod runs on Linux desktops only (X11 or Wayland)");
    std::process::exit(1);
}
