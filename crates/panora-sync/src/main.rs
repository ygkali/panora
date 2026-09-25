// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! `panora-sync`: the device sync service and its command line.
//!
//! `panora-sync run` is what `panora-sync.service` starts. Every other
//! command talks to that running service over its control socket.
//!
//! What the commands print follows `ui.language` (or the locale), like
//! `panora-cli`; `--help`, the manual page and the service's own log stay
//! in English.

#![forbid(unsafe_code)]

use clap::{CommandFactory, Parser, Subcommand};
use panora_core::config::{self, Config};
use panora_core::i18n::{fill, Language, Strings};
use panora_core::sync::control::Status;
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
    let s = strings();
    let result = runtime.block_on(async {
        match cli.command {
            Command::Run => run().await,
            Command::Status => status(s).await,
            Command::Invite => talk(s, Request::Invite).await,
            Command::Pair => talk(s, Request::Pair).await,
            Command::Join {
                link,
                code,
                address,
            } => match (link, code) {
                (Some(link), false) => talk(s, Request::Join { link }).await,
                (None, true) => talk(s, Request::JoinCode { address }).await,
                _ => Err(s.sync_cli_link_or_code.into()),
            },
            Command::Remove { device } => talk(s, Request::Remove { device }).await,
            Command::Leave { yes } => {
                if !yes {
                    println!("{}", s.sync_leave_body);
                    if !ask(&question(s, s.sync_leave_title)).await {
                        return Ok(());
                    }
                }
                talk(s, Request::Leave).await
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

/// The catalogue for `ui.language`, or the locale when the configuration
/// cannot be read.
fn strings() -> &'static Strings {
    Config::load()
        .map(|c| Language::from_config(&c.ui.language))
        .unwrap_or_else(|_| Language::from_environment())
        .strings()
}

/// `text` as a yes/no question with the default (no) marked.
fn question(s: &Strings, text: &str) -> String {
    format!("{text} {} ", s.sync_cli_yes_no)
}

/// Ask on the terminal; only an explicit yes (in English or Turkish)
/// counts.
async fn ask(question: &str) -> bool {
    let question = question.to_string();
    tokio::task::spawn_blocking(move || {
        print!("{question}");
        let _ = std::io::stdout().flush();
        let mut line = String::new();
        std::io::stdin().read_line(&mut line).is_ok() && is_yes(&line)
    })
    .await
    .unwrap_or(false)
}

fn is_yes(answer: &str) -> bool {
    matches!(
        answer.trim().to_lowercase().as_str(),
        "y" | "yes" | "e" | "evet"
    )
}

async fn connect(s: &Strings) -> Result<Client, Failure> {
    Client::connect(&control::socket_path())
        .await
        .map_err(|_| s.sync_cli_not_running.into())
}

async fn status(s: &Strings) -> Result<(), Failure> {
    let mut client = connect(s).await?;
    client.send(&Request::Status).await?;
    match client.next().await? {
        Some(Event::Status { status }) => {
            print!("{}", status_text(s, &status));
            Ok(())
        }
        Some(Event::Error { failure, message }) => Err(failure.describe(s, &message).into()),
        other => Err(format!("unexpected reply: {other:?}").into()),
    }
}

/// What `panora-sync status` prints.
fn status_text(s: &Strings, status: &Status) -> String {
    let mut out = String::new();
    // The fingerprint first: it is hex, while a name could contain `{f}`.
    let me = fill(s.sync_cli_this_device, "f", &status.fingerprint);
    out.push_str(&fill(&me, "name", &status.device_name));
    out.push('\n');
    out.push_str(&fill(s.sync_cli_listening_on, "addr", &status.listen));
    out.push('\n');
    if !status.in_group {
        out.push_str(s.sync_cli_no_group);
        out.push('\n');
        return out;
    }
    let state = match (status.member, status.has_key) {
        (false, _) => s.sync_cli_state_removed,
        (true, false) => s.sync_cli_state_waiting_key,
        (true, true) => s.sync_cli_state_member,
    };
    let group = fill(s.sync_cli_group, "state", state);
    out.push_str(&fill(&group, "n", &status.epoch.to_string()));
    out.push('\n');
    let width = status
        .devices
        .iter()
        .map(|d| d.name.chars().count())
        .max()
        .unwrap_or(0)
        .clamp(8, 32);
    for d in &status.devices {
        let mark = if d.this_device {
            s.sync_this_device
        } else if d.connected {
            s.sync_connected
        } else {
            s.sync_not_connected
        };
        out.push_str(&format!(
            "  {:<width$}  {}  {}\n",
            d.name, d.fingerprint, mark
        ));
    }
    out
}

fn print_qr(s: &Strings, link: &str) {
    match qrcode::QrCode::new(link.as_bytes()) {
        Ok(code) => {
            let art = code
                .render::<qrcode::render::unicode::Dense1x2>()
                .quiet_zone(true)
                .build();
            println!("{art}");
        }
        Err(e) => eprintln!("{}", fill(s.sync_cli_no_qr, "e", &e.to_string())),
    }
}

/// Who the other device is, for a question.
fn who(s: &Strings, name: &str, fingerprint: &str) -> String {
    if name.is_empty() {
        fill(s.sync_cli_device, "f", fingerprint)
    } else {
        fill(&fill(s.sync_compare_named, "f", fingerprint), "name", name)
    }
}

/// The question for an `ask` event.
fn pairing_question(s: &Strings, code: Option<&str>, name: &str, fingerprint: &str) -> String {
    let who = who(s, name, fingerprint);
    let text = match code {
        Some(code) => fill(&fill(s.sync_cli_ask_code, "who", &who), "code", code),
        None => fill(s.sync_cli_ask, "who", &who),
    };
    question(s, &text)
}

/// Whole minutes from now until `expires_at`, at least one.
fn minutes_left(expires_at: i64) -> i64 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    ((expires_at - now + 59) / 60).max(1)
}

/// Send one request and print what comes back until it is done,
/// answering questions from the terminal.
async fn follow(s: &Strings, mut client: Client) -> Result<(), Failure> {
    while let Some(event) = client.next().await? {
        match event {
            Event::Link { link, expires_at } => {
                println!("{}\n\n  panora-sync join '{link}'\n", s.sync_cli_run_join);
                println!("{}\n", s.sync_cli_or_scan);
                print_qr(s, &link);
                let note = fill(
                    s.sync_cli_invite_note,
                    "n",
                    &minutes_left(expires_at).to_string(),
                );
                println!("{note}");
            }
            Event::Listening { .. } => println!("{}", s.sync_cli_listening),
            Event::Joining { fingerprint } => {
                println!("{}", fill(s.sync_joining, "f", &fingerprint))
            }
            Event::AttemptFailed { failure, message } => {
                let reason = failure.describe(s, &message);
                println!("{}", fill(s.sync_attempt_failed, "reason", &reason));
            }
            Event::Code { code } => println!("\n{}\n", fill(s.sync_cli_code, "code", &code)),
            Event::Ask {
                code,
                name,
                fingerprint,
            } => {
                let accept = ask(&pairing_question(s, code.as_deref(), &name, &fingerprint)).await;
                client.send(&Request::Answer { accept }).await?;
            }
            Event::Done { outcome } => {
                println!("{}", outcome_text(s, &outcome));
                return Ok(());
            }
            Event::Error { failure, message } => return Err(failure.describe(s, &message).into()),
            Event::Status { .. } => {}
        }
    }
    Err(s.sync_cli_service_closed.into())
}

/// How a request ended, in one or two lines.
fn outcome_text(s: &Strings, outcome: &Outcome) -> String {
    match outcome {
        Outcome::DeviceJoined { name, .. } => fill(s.sync_device_joined, "name", name),
        Outcome::JoinedGroup => format!("{}. {}", s.sync_joined, s.sync_joined_body),
        Outcome::Removed { name, .. } => {
            format!(
                "{}. {}",
                fill(s.sync_removed, "name", name),
                s.sync_cli_key_replaced
            )
        }
        Outcome::Left => format!("{}. {}", s.sync_left, s.sync_cli_left_hint),
    }
}

async fn talk(s: &Strings, request: Request) -> Result<(), Failure> {
    let mut client = connect(s).await?;
    client.send(&request).await?;
    follow(s, client).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use panora_core::sync::control::DeviceRow;

    fn status(in_group: bool) -> Status {
        Status {
            device_name: "desktop".into(),
            fingerprint: "3f2a-91c0-7b44-e1d8-0a5c".into(),
            listen: "[::]:47100".into(),
            in_group,
            member: in_group,
            has_key: in_group,
            epoch: 3,
            devices: vec![
                DeviceRow {
                    name: "desktop".into(),
                    fingerprint: "3f2a-91c0-7b44-e1d8-0a5c".into(),
                    this_device: true,
                    connected: false,
                },
                DeviceRow {
                    name: "laptop".into(),
                    fingerprint: "77aa-0011-2233-4455-6677".into(),
                    this_device: false,
                    connected: true,
                },
            ],
        }
    }

    #[test]
    fn status_reads_in_both_languages() {
        let en = status_text(Language::English.strings(), &status(true));
        assert!(
            en.starts_with("This device: desktop (3f2a-91c0-7b44-e1d8-0a5c)\n"),
            "{en}"
        );
        assert!(en.contains("Group: member, roster epoch 3"), "{en}");
        assert!(
            en.contains("  laptop    77aa-0011-2233-4455-6677  Connected"),
            "{en}"
        );
        let tr = status_text(Language::Turkish.strings(), &status(true));
        assert!(
            tr.starts_with("Bu cihaz: desktop (3f2a-91c0-7b44-e1d8-0a5c)\n"),
            "{tr}"
        );
        assert!(tr.contains("Grup: üye, cihaz listesi sürümü 3"), "{tr}");
        assert!(tr.contains("Bağlı"), "{tr}");
        let alone = status_text(Language::Turkish.strings(), &status(false));
        assert!(alone.contains("panora-sync invite"), "{alone}");
        assert!(!alone.contains("Grup:"), "{alone}");
    }

    #[test]
    fn a_name_cannot_rewrite_the_fingerprint_it_is_shown_with() {
        let s = Language::English.strings();
        let mut sneaky = status(false);
        sneaky.device_name = "{f}".into();
        assert!(status_text(s, &sneaky).starts_with("This device: {f} (3f2a-"));
        let q = pairing_question(s, Some("042 917"), "{f}", "77aa-0011-2233-4455-6677");
        assert!(
            q.contains("{f}, fingerprint 77aa-0011-2233-4455-6677"),
            "{q}"
        );
    }

    #[test]
    fn questions_and_outcomes_in_both_languages() {
        let en = Language::English.strings();
        let tr = Language::Turkish.strings();
        assert_eq!(
            pairing_question(en, Some("042 917"), "", "77aa"),
            "Does the other screen show 042 917? Pair with device 77aa? [y/N] "
        );
        assert_eq!(
            pairing_question(tr, Some("042 917"), "laptop", "77aa"),
            "Diğer ekranda 042 917 mi görünüyor? laptop, parmak izi 77aa ile eşleşilsin mi? [e/H] "
        );
        assert!(outcome_text(tr, &Outcome::Left).starts_with("Bu cihaz gruptan ayrıldı."));
        assert!(outcome_text(
            en,
            &Outcome::Removed {
                name: "laptop".into(),
                fingerprint: "77aa".into()
            }
        )
        .contains("group key has been replaced"));
    }

    #[test]
    fn only_an_explicit_yes_counts() {
        for yes in ["y", "Yes\n", " e ", "EVET"] {
            assert!(is_yes(yes), "{yes:?}");
        }
        for no in ["", "n", "h", "hayır", "no", "yess"] {
            assert!(!is_yes(no), "{no:?}");
        }
    }

    #[test]
    fn minutes_round_up_and_never_reach_zero() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        assert_eq!(minutes_left(now + 600), 10);
        assert_eq!(minutes_left(now + 61), 2);
        assert_eq!(minutes_left(now - 5), 1);
    }
}
