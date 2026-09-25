// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! The control socket of `panora-sync run`: how the command line and the
//! GUI ask the running service to pair, list or remove devices. The
//! messages are in [`panora_core::sync::control`]; this is the service
//! side and an async client. JSON lines over a 0600 Unix socket in the
//! user's runtime directory; connections from another user are refused.

use crate::error::{Error, Result};
use crate::invite::Invitation;
use crate::node::{Node, PairEvent};
use crate::pairing::JoinRequest;
pub use panora_core::sync::control::{
    socket_path, DeviceRow, Event, Failure, Outcome, Request, Status, MAX_LINE,
};
use serde::Serialize;
use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{mpsc, watch};
use tracing::{debug, warn};

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

    // Later lines are answers to questions; the end of the stream means
    // the client went away (Ctrl+C, the GUI's Back button), which cancels.
    let (answers_tx, mut answers) = mpsc::unbounded_channel();
    let (gone_tx, mut gone) = watch::channel(false);
    tokio::spawn(async move {
        while let Ok(Some(line)) = read_line(&mut reader).await {
            if let Ok(Request::Answer { accept }) = serde_json::from_str(&line) {
                if answers_tx.send(accept).is_err() {
                    break;
                }
            }
        }
        let _ = gone_tx.send(true);
    });

    let result = run(&node, request, &mut write, &mut answers, &mut gone).await;
    let last = match result {
        Ok(Some(outcome)) => Event::Done { outcome },
        Ok(None) => return Ok(()),
        Err(e) => Event::Error {
            failure: e.failure(),
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
    gone: &mut watch::Receiver<bool>,
) -> Result<Option<Outcome>> {
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
            // Every way out of this loop reaches `close_window`: a failed
            // write breaks with an error rather than returning early.
            let result = loop {
                tokio::select! {
                    biased;
                    _ = client_gone(gone) => break Err(Error::Cancelled),
                    _ = &mut expiry => break Err(Error::Expired("the invitation expired")),
                    event = events.recv() => match event {
                        Some(PairEvent::Joined(request)) => break Ok(Some(joined(&request))),
                        Some(PairEvent::Failed { failure, message }) => {
                            if let Err(e) = write_json(out, &Event::AttemptFailed { failure, message }).await {
                                break Err(e);
                            }
                        }
                        Some(_) => {}
                        None => break Err(Error::Expired("the pairing window closed")),
                    },
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
                &Event::Listening {
                    expires_at: crate::node::now() + crate::invite::CODE_WINDOW_SECS,
                },
            )
            .await?;
            let expiry =
                tokio::time::sleep(Duration::from_secs(crate::invite::CODE_WINDOW_SECS as u64));
            tokio::pin!(expiry);
            let result = loop {
                tokio::select! {
                    biased;
                    _ = client_gone(gone) => break Err(Error::Cancelled),
                    _ = &mut expiry => break Err(Error::Expired("nobody paired in time")),
                    event = events.recv() => match event {
                        Some(PairEvent::Code(code)) => {
                            if let Err(e) = write_json(out, &Event::Code { code: code.to_string() }).await {
                                break Err(e);
                            }
                        }
                        Some(PairEvent::Request { code, request, reply }) => {
                            let ask = Event::Ask {
                                code: code.map(|c| c.to_string()),
                                name: request.name.clone(),
                                fingerprint: request.identity.fingerprint(),
                            };
                            if let Err(e) = write_json(out, &ask).await {
                                let _ = reply.send(false);
                                break Err(e);
                            }
                            // No answer (the client went away) is a no.
                            let accept = answers.recv().await.unwrap_or(false);
                            let _ = reply.send(accept);
                        }
                        Some(PairEvent::Joined(request)) => break Ok(Some(joined(&request))),
                        Some(PairEvent::Failed { failure, message }) => {
                            if let Err(e) = write_json(out, &Event::AttemptFailed { failure, message }).await {
                                break Err(e);
                            }
                        }
                        None => break Err(Error::Expired("the pairing window closed")),
                    },
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
                &Event::Joining {
                    fingerprint: invitation.inviter.fingerprint(),
                },
            )
            .await?;
            tokio::select! {
                biased;
                _ = client_gone(gone) => Err(Error::Cancelled),
                result = node.join(invitation) => result.map(|()| Some(Outcome::JoinedGroup)),
            }
        }
        Request::JoinCode { address } => {
            let address = address
                .map(|a| {
                    a.parse::<SocketAddr>()
                        .map_err(|_| Error::Address("the address must be ip:port"))
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
                    biased;
                    _ = client_gone(gone) => return Err(Error::Cancelled),
                    result = &mut joining => {
                        result?;
                        return Ok(Some(Outcome::JoinedGroup));
                    }
                    Some(event) = to_client.recv() => write_json(out, &event).await?,
                }
            }
        }
        Request::Remove { device } => {
            let member = node.remove(&device)?;
            Ok(Some(Outcome::Removed {
                name: member.name,
                fingerprint: member.identity.fingerprint(),
            }))
        }
        Request::Leave => {
            node.leave()?;
            Ok(Some(Outcome::Left))
        }
        Request::Answer { .. } => Err(Error::Protocol("nothing to answer")),
    }
}

/// Resolves once the client has closed its side of the connection.
async fn client_gone(gone: &mut watch::Receiver<bool>) {
    let _ = gone.wait_for(|gone| *gone).await;
}

fn joined(request: &JoinRequest) -> Outcome {
    Outcome::DeviceJoined {
        name: request.name.clone(),
        fingerprint: request.identity.fingerprint(),
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
