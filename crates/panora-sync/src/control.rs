// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! The control socket of `panora-sync run`: how the command line (and
//! later the GUI) asks the running service to pair, list or remove
//! devices. JSON lines over a 0600 Unix socket in the user's runtime
//! directory; connections from another user are refused.
//!
//! A client sends one request, then reads events until `done` or
//! `error`. Pairing questions arrive as `ask`, answered with an `answer`
//! request on the same connection.

use crate::error::{Error, Result};
use crate::invite::Invitation;
use crate::node::{Node, PairEvent, Status};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc;
use tracing::{debug, warn};

/// Longest control line accepted.
const MAX_LINE: u64 = 64 * 1024;

/// Where the service listens.
pub fn socket_path() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(panora_core::config::data_dir)
        .join("panora-sync.sock")
}

/// What a client asks.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Request {
    /// This device, its group, its connections.
    Status,
    /// Invite a device; replies with the link.
    Invite,
    /// Accept one device by comparing a code.
    Pair,
    /// Join with an invitation link.
    Join {
        /// The link.
        link: String,
    },
    /// Join by comparing a code, with the device at `address` or the only
    /// one mDNS finds.
    JoinCode {
        /// `ip:port`, if known.
        address: Option<String>,
    },
    /// Remove a device (name, or at least four characters of its
    /// fingerprint).
    Remove {
        /// Which one.
        device: String,
    },
    /// Leave the group on this device.
    Leave,
    /// The user's answer to the last `ask`.
    Answer {
        /// Yes or no.
        accept: bool,
    },
}

/// What the service reports.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    /// Reply to `status`.
    Status {
        /// The report.
        status: Status,
    },
    /// The invitation link to show (as text and QR code).
    Link {
        /// `panora-pair:1?...`
        link: String,
        /// Unix time it stops working.
        expires_at: i64,
    },
    /// Progress worth showing.
    Waiting {
        /// What is happening.
        message: String,
    },
    /// The code to show, on the inviting device.
    Code {
        /// `042 917`
        code: String,
    },
    /// A question for the user; answer with `answer`.
    Ask {
        /// The code to compare, if any.
        code: Option<String>,
        /// The other device's name, if known.
        name: String,
        /// The other device's fingerprint.
        fingerprint: String,
    },
    /// Finished.
    Done {
        /// What happened.
        message: String,
    },
    /// Failed.
    Error {
        /// Why.
        message: String,
    },
}

/// Read one line of at most `MAX_LINE` bytes; `None` at end of stream.
async fn read_line<R: AsyncBufRead + Unpin>(reader: &mut R) -> Result<Option<String>> {
    let mut line = String::new();
    let n = reader.take(MAX_LINE).read_line(&mut line).await?;
    if n == 0 {
        return Ok(None);
    }
    if !line.ends_with('\n') {
        return Err(Error::Protocol("control line too long"));
    }
    Ok(Some(line))
}

async fn write_json<W: AsyncWriteExt + Unpin, T: Serialize>(
    writer: &mut W,
    value: &T,
) -> Result<()> {
    let mut line = serde_json::to_vec(value)?;
    line.push(b'\n');
    writer.write_all(&line).await?;
    writer.flush().await?;
    Ok(())
}

/// Serve the control socket at `path` until the listener fails.
pub async fn serve(node: Node, path: &Path) -> Result<()> {
    if let Ok(meta) = std::fs::symlink_metadata(path) {
        use std::os::unix::fs::FileTypeExt;
        if meta.file_type().is_socket() {
            let _ = std::fs::remove_file(path);
        }
    }
    let listener = UnixListener::bind(path)?;
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    let own_uid = {
        use std::os::unix::fs::MetadataExt;
        std::fs::metadata(path)?.uid()
    };
    loop {
        let (stream, _) = listener.accept().await?;
        match stream.peer_cred() {
            Ok(cred) if cred.uid() == own_uid => {}
            _ => {
                warn!("refused a control connection from another user");
                continue;
            }
        }
        let node = node.clone();
        tokio::spawn(async move {
            if let Err(e) = handle(node, stream).await {
                debug!(error = %e, "control connection ended");
            }
        });
    }
}

async fn handle(node: Node, stream: UnixStream) -> Result<()> {
    let (read, mut write) = stream.into_split();
    let mut reader = BufReader::new(read);
    let first = tokio::time::timeout(Duration::from_secs(10), read_line(&mut reader))
        .await
        .map_err(|_| Error::Protocol("no request"))??
        .ok_or(Error::Protocol("no request"))?;
    let request: Request = serde_json::from_str(&first)?;

    // Later lines are answers to questions.
    let (answers_tx, mut answers) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        while let Ok(Some(line)) = read_line(&mut reader).await {
            if let Ok(Request::Answer { accept }) = serde_json::from_str(&line) {
                if answers_tx.send(accept).is_err() {
                    return;
                }
            }
        }
    });

    let result = run(&node, request, &mut write, &mut answers).await;
    let last = match result {
        Ok(Some(message)) => Event::Done { message },
        Ok(None) => return Ok(()),
        Err(e) => Event::Error {
            message: e.to_string(),
        },
    };
    write_json(&mut write, &last).await
}

async fn run<W: AsyncWriteExt + Unpin>(
    node: &Node,
    request: Request,
    out: &mut W,
    answers: &mut mpsc::UnboundedReceiver<bool>,
) -> Result<Option<String>> {
    match request {
        Request::Status => {
            write_json(
                out,
                &Event::Status {
                    status: node.status(),
                },
            )
            .await?;
            Ok(None)
        }
        Request::Invite => {
            let (invitation, mut events, epoch) = node.invite().await?;
            write_json(
                out,
                &Event::Link {
                    link: invitation.to_uri(),
                    expires_at: invitation.expires_at,
                },
            )
            .await?;
            let expiry = tokio::time::sleep(Duration::from_secs(
                crate::invite::INVITATION_TTL_SECS as u64,
            ));
            tokio::pin!(expiry);
            let result = loop {
                tokio::select! {
                    event = events.recv() => match event {
                        Some(PairEvent::Joined(request)) => {
                            break Ok(Some(format!("{} joined the group", request.name)));
                        }
                        Some(PairEvent::Failed(message)) => {
                            write_json(out, &Event::Waiting { message: format!("an attempt failed: {message}") }).await?;
                        }
                        Some(_) => {}
                        None => break Err(Error::Invitation("the pairing window closed")),
                    },
                    _ = &mut expiry => break Err(Error::Invitation("the invitation expired")),
                    // The client went away (Ctrl+C): stop inviting.
                    None = answers.recv() => break Err(Error::Cancelled),
                }
            };
            if result.is_err() {
                node.close_window(epoch).await;
            }
            result
        }
        Request::Pair => {
            let (mut events, epoch) = node.open_code_window().await?;
            write_json(
                out,
                &Event::Waiting {
                    message: "waiting for a device; on it, run: panora-sync join --code".into(),
                },
            )
            .await?;
            let expiry =
                tokio::time::sleep(Duration::from_secs(crate::invite::CODE_WINDOW_SECS as u64));
            tokio::pin!(expiry);
            let result = loop {
                tokio::select! {
                    event = events.recv() => match event {
                        Some(PairEvent::Code(code)) => {
                            write_json(out, &Event::Code { code: code.to_string() }).await?;
                        }
                        Some(PairEvent::Request { code, request, reply }) => {
                            write_json(out, &Event::Ask {
                                code: code.map(|c| c.to_string()),
                                name: request.name.clone(),
                                fingerprint: request.identity.fingerprint(),
                            }).await?;
                            let accept = answers.recv().await.unwrap_or(false);
                            let _ = reply.send(accept);
                        }
                        Some(PairEvent::Joined(request)) => {
                            break Ok(Some(format!("{} joined the group", request.name)));
                        }
                        Some(PairEvent::Failed(message)) => {
                            write_json(out, &Event::Waiting { message: format!("an attempt failed: {message}") }).await?;
                        }
                        None => break Err(Error::Invitation("the pairing window closed")),
                    },
                    _ = &mut expiry => break Err(Error::Invitation("nobody paired in time")),
                    None = answers.recv() => break Err(Error::Cancelled),
                }
            };
            if result.is_err() {
                node.close_window(epoch).await;
            }
            result
        }
        Request::Join { link } => {
            let invitation = Invitation::parse(&link)?;
            write_json(
                out,
                &Event::Waiting {
                    message: format!(
                        "joining the group of device {}",
                        invitation.inviter.fingerprint()
                    ),
                },
            )
            .await?;
            node.join(invitation).await?;
            Ok(Some("joined the group; syncing starts now".into()))
        }
        Request::JoinCode { address } => {
            let address = address
                .map(|a| {
                    a.parse::<SocketAddr>()
                        .map_err(|_| Error::Invitation("the address must be ip:port"))
                })
                .transpose()?;
            let (asks, mut to_client) = mpsc::unbounded_channel();
            let joining = node.join_by_code(address, |code, inviter| {
                let _ = asks.send(Event::Ask {
                    code: Some(code.to_string()),
                    name: String::new(),
                    fingerprint: inviter.fingerprint(),
                });
                async move { answers.recv().await.unwrap_or(false) }
            });
            tokio::pin!(joining);
            loop {
                tokio::select! {
                    result = &mut joining => {
                        result?;
                        return Ok(Some("joined the group; syncing starts now".into()));
                    }
                    Some(event) = to_client.recv() => write_json(out, &event).await?,
                }
            }
        }
        Request::Remove { device } => {
            let member = node.remove(&device)?;
            Ok(Some(format!(
                "removed {} ({}); the group key has been replaced",
                member.name,
                member.identity.fingerprint()
            )))
        }
        Request::Leave => {
            node.leave()?;
            Ok(Some(
                "left the group on this device; remove it from another device too".into(),
            ))
        }
        Request::Answer { .. } => Err(Error::Protocol("nothing to answer")),
    }
}

/// A connection to the control socket, for the command line.
pub struct Client {
    reader: BufReader<tokio::net::unix::OwnedReadHalf>,
    writer: tokio::net::unix::OwnedWriteHalf,
}

impl Client {
    /// Connect to the running service.
    pub async fn connect(path: &Path) -> Result<Self> {
        let stream = UnixStream::connect(path).await.map_err(|e| {
            Error::Transport(format!(
                "panora-sync is not running ({e}); start it with: systemctl --user start panora-sync"
            ))
        })?;
        let (read, writer) = stream.into_split();
        Ok(Self {
            reader: BufReader::new(read),
            writer,
        })
    }

    /// Send a request (or an answer).
    pub async fn send(&mut self, request: &Request) -> Result<()> {
        write_json(&mut self.writer, request).await
    }

    /// The next event; `None` when the service closed the connection.
    pub async fn next(&mut self) -> Result<Option<Event>> {
        match read_line(&mut self.reader).await? {
            Some(line) => Ok(Some(serde_json::from_str(&line)?)),
            None => Ok(None),
        }
    }
}
