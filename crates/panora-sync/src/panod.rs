// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! Talking to `panod` over its own socket (protocol v3): the change feed,
//! applying other devices' records, and being told when the history
//! changed. The calls are blocking and run on tokio's blocking pool.

use crate::error::{Error, Result};
use panora_core::ipc::{v3, Event, Request, ResponseData};
use panora_core::sync::{SyncCursor, SyncRecord, SyncScope};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::sync::watch;

/// A client for one `panod` socket.
#[derive(Debug, Clone)]
pub struct Panod {
    socket: PathBuf,
}

fn ipc_err(e: impl std::fmt::Display) -> Error {
    Error::Panod(e.to_string())
}

fn connect(socket: &Path) -> Result<UnixStream> {
    let stream = UnixStream::connect(socket)
        .map_err(|e| Error::Panod(format!("not reachable at {}: {e}", socket.display())))?;
    v3::write_magic_blocking(&stream).map_err(ipc_err)?;
    Ok(stream)
}

fn call_blocking(socket: &Path, request: Request) -> Result<ResponseData> {
    let stream = connect(socket)?;
    stream.set_read_timeout(Some(Duration::from_secs(120)))?;
    v3::write_request_blocking(&stream, request).map_err(ipc_err)?;
    v3::read_response_blocking(&stream)
        .map_err(ipc_err)?
        .into_result()
        .map_err(ipc_err)
}

impl Panod {
    /// A client for the daemon listening on `socket`.
    pub fn new(socket: PathBuf) -> Self {
        Self { socket }
    }

    async fn call(&self, request: Request) -> Result<ResponseData> {
        let socket = self.socket.clone();
        tokio::task::spawn_blocking(move || call_blocking(&socket, request))
            .await
            .map_err(ipc_err)?
    }

    /// One page of the change feed after `since`.
    pub async fn changes(
        &self,
        since: SyncCursor,
        limit: usize,
        scope: SyncScope,
    ) -> Result<(Vec<SyncRecord>, SyncCursor, bool)> {
        match self
            .call(Request::SyncChanges {
                since,
                limit,
                scope,
            })
            .await?
        {
            ResponseData::SyncChanges {
                records,
                next,
                more,
            } => Ok((records, next, more)),
            other => Err(Error::Panod(format!("unexpected reply {other:?}"))),
        }
    }

    /// Apply records, split into requests `panod` accepts (at most
    /// `v3::MAX_FDS_PER_FRAME` payloads each). Returns (applied, ignored,
    /// rejected) summed over all of them.
    pub async fn apply(&self, records: Vec<SyncRecord>) -> Result<(usize, usize, usize)> {
        let mut totals = (0, 0, 0);
        let mut batch = Vec::new();
        let mut payloads = 0;
        let mut chunks = Vec::new();
        for record in records {
            let n = record.payloads.len();
            if !batch.is_empty() && payloads + n > v3::MAX_FDS_PER_FRAME {
                chunks.push(std::mem::take(&mut batch));
                payloads = 0;
            }
            payloads += n;
            batch.push(record);
        }
        if !batch.is_empty() {
            chunks.push(batch);
        }
        for records in chunks {
            match self.call(Request::SyncApply { records }).await? {
                ResponseData::SyncApplied {
                    applied,
                    ignored,
                    rejected,
                } => {
                    totals.0 += applied;
                    totals.1 += ignored;
                    totals.2 += rejected;
                }
                other => return Err(Error::Panod(format!("unexpected reply {other:?}"))),
            }
        }
        Ok(totals)
    }

    /// Whether the daemon is not taking entries now (private mode, or the
    /// second-layer lock).
    pub async fn paused(&self) -> Result<bool> {
        match self.call(Request::Status).await? {
            ResponseData::Status(status) => {
                Ok(status.private_mode || status.locked || status.app_locked)
            }
            other => Err(Error::Panod(format!("unexpected reply {other:?}"))),
        }
    }

    /// Ask `panod` to re-read the configuration.
    pub async fn reload_config(&self) -> Result<()> {
        self.call(Request::ReloadConfig).await.map(|_| ())
    }

    /// Follow `panod`'s history revision. A background thread keeps a
    /// `Subscribe` stream open (reconnecting when the daemon restarts) and
    /// stops once every receiver is gone.
    pub fn watch(&self) -> watch::Receiver<u64> {
        let (tx, rx) = watch::channel(0u64);
        let socket = self.socket.clone();
        std::thread::Builder::new()
            .name("panod-events".into())
            .spawn(move || loop {
                if tx.is_closed() {
                    return;
                }
                if let Ok(stream) = connect(&socket) {
                    if v3::write_request_blocking(&stream, Request::Subscribe).is_ok() {
                        while let Ok(Event::Changed { revision }) = v3::read_event_blocking(&stream)
                        {
                            if tx.send(revision).is_err() {
                                return;
                            }
                        }
                    }
                }
                // The daemon is down or restarting. A new subscription
                // starts with the current revision, which wakes the
                // sessions once it is back.
                std::thread::sleep(Duration::from_secs(2));
            })
            .expect("spawning a thread");
        rx
    }
}
