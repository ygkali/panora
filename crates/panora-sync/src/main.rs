// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! `panora-sync`: the device sync service and its command line.
//!
//! `panora-sync run` is what `panora-sync.service` starts. Every other
//! command talks to that running service over its control socket.

#![forbid(unsafe_code)]

use clap::{CommandFactory, Parser, Subcommand};
use panora_core::config::{self, Config};
use panora_core::sync::SyncScope;
use panora_sync::control::{self, Client, Event, Outcome, Request};
use panora_sync::group::validate_name;
use panora_sync::node::{Node, NodeConfig};
use panora_sync::{keyring, SyncState};
use std::io::Write as _;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

/// Sync the clipboard history between your own devices on the local
/// network (experimental).
#[derive(Parser)]
#[command(name = "panora-sync", version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run the sync service (what panora-sync.service starts).
    Run,
    /// Show this device, its group and which devices are connected.
    Status,
    /// Invite a device: prints a link and a QR code to use on it.
    Invite,
    /// Accept a device by comparing a six-digit code on both screens.
    Pair,
    /// Join a group, with an invitation link or by comparing a code.
    Join {
        /// The invitation link (panora-pair:1?...).
        link: Option<String>,
        /// Compare a code with a device running `panora-sync pair`.
        #[arg(long, conflicts_with = "link")]
        code: bool,
        /// That device's ip:port, when mDNS cannot find it.
        #[arg(long, requires = "code")]
        address: Option<String>,
    },
    /// Remove a device from the group (its name, or the start of its
    /// fingerprint). The group key is replaced.
    Remove {
        /// Name or fingerprint.
        device: String,
    },
    /// Leave the group on this device.
    Leave {
        /// Do not ask for confirmation.
        #[arg(long)]
        yes: bool,
    },
    /// Write the manual page into DIR.
    #[command(hide = true)]
    Man {
        /// Output directory.
        dir: PathBuf,
    },
}

type Failure = Box<dyn std::error::Error>;

fn main() -> ExitCode {
    let cli = Cli::parse();
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("panora-sync: {e}");
            return ExitCode::FAILURE;
        }
    };
    let result = runtime.block_on(async {
        match cli.command {
            Command::Run => run().await,
            Command::Status => status().await,
            Command::Invite => invite().await,
            Command::Pair => pair().await,
            Command::Join {
                link,
                code,
                address,
            } => match (link, code) {
                (Some(link), false) => talk(Request::Join { link }).await,
                (None, true) => talk(Request::JoinCode { address }).await,
                _ => Err("give an invitation link, or --code".into()),
            },
            Command::Remove { device } => talk(Request::Remove { device }).await,
            Command::Leave { yes } => {
                if !yes
                    && !ask("Leave the sync group on this device? History stays here. [y/N] ").await
                {
                    return Ok(());
                }
                talk(Request::Leave).await
            }
            Command::Man { dir } => man(dir),
        }
    });
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("panora-sync: {e}");
            ExitCode::FAILURE
        }
    }
}

fn man(dir: PathBuf) -> Result<(), Failure> {
    let mut file = std::fs::File::create(dir.join("panora-sync.1"))?;
    clap_mangen::Man::new(Cli::command()).render(&mut file)?;
    Ok(())
}

fn hostname() -> String {
    std::fs::read_to_string("/proc/sys/kernel/hostname")
        .ok()
        .map(|h| h.trim().to_string())
        .filter(|h| validate_name(h).is_ok())
        .unwrap_or_else(|| "Panora device".to_string())
}

/// The id `panod` stamps on entries; it writes the file on its first
/// start, so wait a little for it.
async fn device_id() -> Result<String, Failure> {
    let path = config::data_dir().join("device-id");
    for _ in 0..30 {
        if let Ok(id) = std::fs::read_to_string(&path) {
            let id = id.trim().to_string();
            if !id.is_empty() {
                return Ok(id);
            }
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    Err(format!("{} not found; start panod first", path.display()).into())
}

async fn run() -> Result<(), Failure> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    let cfg = Config::load()?;
    let device_id = device_id().await?;
    let dir = config::data_dir().join("sync");
    std::fs::create_dir_all(&dir)?;
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
    }
    let state_path = dir.join("state.bin");
    let key = keyring::load_or_create_state_key().await?;
    let state = match SyncState::load(&state_path, &key)? {
        Some(state) => state,
        None => {
            let state = SyncState::new(&hostname())?;
            state.save(&state_path, &key)?;
            state
        }
    };
    let peers: Vec<SocketAddr> = cfg
        .sync
        .peers
        .iter()
        .filter_map(|p| p.parse().ok())
        .collect();
    let node_config = |listen: SocketAddr| NodeConfig {
        listen,
        peers: peers.clone(),
        discovery: cfg.sync.discovery,
        panod_socket: config::socket_path(),
        state_path: state_path.clone(),
        device_id: device_id.clone(),
        scope: SyncScope {
            pinned_only: cfg.sync.pinned_only,
            text_only: cfg.sync.text_only,
        },
        manage_config: true,
    };
    // Dual-stack where IPv6 exists, IPv4 otherwise.
    let port = cfg.sync.port;
    let v6 = SocketAddr::from(([0u16; 8], port));
    let listen = if std::net::UdpSocket::bind(v6).is_ok() {
        v6
    } else {
        SocketAddr::from(([0u8; 4], port))
    };
    let node = Node::start(node_config(listen), state, key).await?;
    let socket = control::socket_path();
    let serving = node.clone();
    let control_path = socket.clone();
    tokio::spawn(async move {
        if let Err(e) = control::serve(serving, &control_path).await {
            tracing::error!(error = %e, "control socket failed");
        }
    });
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = term.recv() => {}
    }
    node.shutdown();
    let _ = std::fs::remove_file(&socket);
    Ok(())
}

async fn ask(question: &str) -> bool {
    let question = question.to_string();
    tokio::task::spawn_blocking(move || {
        print!("{question}");
        let _ = std::io::stdout().flush();
        let mut line = String::new();
        std::io::stdin().read_line(&mut line).is_ok()
            && matches!(
                line.trim().to_lowercase().as_str(),
                "y" | "yes" | "e" | "evet"
            )
    })
    .await
    .unwrap_or(false)
}

async fn status() -> Result<(), Failure> {
    let mut client = Client::connect(&control::socket_path()).await?;
    client.send(&Request::Status).await?;
    match client.next().await? {
        Some(Event::Status { status }) => {
            println!(
                "This device: {} ({})",
                status.device_name, status.fingerprint
            );
            println!("Listening on: {}", status.listen);
            if !status.in_group {
                println!("Not in a sync group. Start one with `panora-sync invite`.");
                return Ok(());
            }
            let state = match (status.member, status.has_key) {
                (false, _) => "not a member (removed, or waiting to be added again)",
                (true, false) => "member, waiting for the current key",
                (true, true) => "member",
            };
            println!("Group: {state}, roster epoch {}", status.epoch);
            for d in status.devices {
                let mark = if d.this_device {
                    "this device"
                } else if d.connected {
                    "connected"
                } else {
                    "not connected"
                };
                println!("  {:<24} {}  {}", d.name, d.fingerprint, mark);
            }
            Ok(())
        }
        Some(Event::Error { message, .. }) => Err(message.into()),
        other => Err(format!("unexpected reply: {other:?}").into()),
    }
}

fn print_qr(link: &str) {
    match qrcode::QrCode::new(link.as_bytes()) {
        Ok(code) => {
            let art = code
                .render::<qrcode::render::unicode::Dense1x2>()
                .quiet_zone(true)
                .build();
            println!("{art}");
        }
        Err(e) => eprintln!("(no QR code: {e})"),
    }
}

/// Send one request and print what comes back until it is done,
/// answering questions from the terminal.
async fn follow(mut client: Client) -> Result<(), Failure> {
    while let Some(event) = client.next().await? {
        match event {
            Event::Link { link, .. } => {
                println!("On the other device, run:\n\n  panora-sync join '{link}'\n");
                println!("or scan this code with it:\n");
                print_qr(&link);
                println!("The invitation works once, for 10 minutes. Waiting...");
            }
            Event::Listening { .. } => {
                println!("Waiting for a device; on it, run: panora-sync join --code");
            }
            Event::Joining { fingerprint } => {
                println!("Joining the group of device {fingerprint}...");
            }
            Event::AttemptFailed { message, .. } => println!("An attempt failed: {message}"),
            Event::Code { code } => println!("\nCode: {code}\n"),
            Event::Ask {
                code,
                name,
                fingerprint,
            } => {
                let who = if name.is_empty() {
                    fingerprint
                } else {
                    format!("{name} ({fingerprint})")
                };
                let question = match code {
                    Some(code) => {
                        format!("Does the other screen show {code}? Pair with {who}? [y/N] ")
                    }
                    None => format!("Pair with {who}? [y/N] "),
                };
                let accept = ask(&question).await;
                client.send(&Request::Answer { accept }).await?;
            }
            Event::Done { outcome } => {
                println!("{}", outcome_text(&outcome));
                return Ok(());
            }
            Event::Error { message, .. } => return Err(message.into()),
            Event::Status { .. } => {}
        }
    }
    Err("the sync service closed the connection".into())
}

fn outcome_text(outcome: &Outcome) -> String {
    match outcome {
        Outcome::DeviceJoined { name, .. } => format!("{name} joined the group"),
        Outcome::JoinedGroup => "Joined the group; syncing starts now".into(),
        Outcome::Removed { name, fingerprint } => {
            format!("Removed {name} ({fingerprint}); the group key has been replaced")
        }
        Outcome::Left => "Left the group on this device; remove it from another device too".into(),
    }
}

async fn talk(request: Request) -> Result<(), Failure> {
    let mut client = Client::connect(&control::socket_path()).await?;
    client.send(&request).await?;
    follow(client).await
}

async fn invite() -> Result<(), Failure> {
    talk(Request::Invite).await
}

async fn pair() -> Result<(), Failure> {
    talk(Request::Pair).await
}
