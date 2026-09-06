// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

#![allow(clippy::arc_with_non_send_sync)]

//! panod executable: starts the display backend, encrypted store and
//! versioned Unix-socket IPC service.

use panod::backend::wayland::WaylandBackend;
use panod::backend::x11::X11Backend;
use panod::daemon::Daemon;
use panod::gnome::{GnomeBridge, BUS_NAME, OBJECT_PATH};
use panod::ipc::{
    decode, encode, Request, Response, ResponseData, StatusData, MAX_FRAME_BYTES,
    MAX_REQUESTS_PER_CONNECTION,
};
use panod::keyring::load_or_create_master_key;
use panora_core::backend::ClipboardBackend;
use panora_core::config::{config_path, data_dir, socket_path, Config};
use panora_core::storage::{BlobStore, Cipher, Database};
use panora_core::sync::NoopSync;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tracing::{error, info, warn};

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let local = tokio::task::LocalSet::new();
    local.run_until(run_app()).await
}

async fn run_app() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "info".into()))
        .compact()
        .init();

    let config =
        Config::load().map_err(|e| anyhow::anyhow!("config {}: {e}", config_path().display()))?;
    let backend: Arc<dyn ClipboardBackend> = match std::env::var("XDG_SESSION_TYPE").as_deref() {
        Ok("x11") => Arc::new(
            X11Backend::connect().map_err(|e| anyhow::anyhow!("X11 backend unavailable: {e}"))?,
        ),
        _ => Arc::new(
            WaylandBackend::connect()
                .map_err(|e| anyhow::anyhow!("Wayland data-control unavailable: {e}"))?,
        ),
    };
    let data_root = data_dir();
    ensure_private_directory(&data_root)?;
    let master = load_or_create_master_key()
        .await
        .map_err(|e| anyhow::anyhow!("keyring unavailable: {e}"))?;
    let db = Database::open(data_root.join("history.db"), Cipher::new(&master))?;
    let blobs = BlobStore::open(data_root.join("blobs"), Cipher::new(&master))?;
    let device_id = load_or_create_device_id()?;
    let daemon = Arc::new(Daemon::new(
        backend,
        db,
        blobs,
        config,
        Arc::new(NoopSync),
        device_id,
    ));

    let bridge_daemon = daemon.clone();
    tokio::task::spawn_local(async move {
        if let Err(e) = run_gnome_bridge(bridge_daemon).await {
            tracing::warn!(error = %e, "GNOME bridge unavailable; install/enable the extension for GNOME Wayland capture");
        }
    });

    let path = socket_path();
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let _ = tokio::fs::remove_file(&path).await;
    let listener = UnixListener::bind(&path)?;
    set_socket_permissions(&path)?;
    let socket_uid = socket_owner_uid(&path)?;
    info!(socket = %path.display(), uid = socket_uid, "panod IPC service ready");

    let (shutdown_tx, shutdown_rx) = tokio::sync::mpsc::channel(1);
    let capture = daemon.clone();
    tokio::task::spawn_local(async move {
        if let Err(e) = capture.run(shutdown_rx).await {
            error!(error = %e, "capture loop stopped");
        }
    });

    let server = daemon.clone();
    let accept_loop = async move {
        loop {
            let (stream, _) = listener.accept().await?;
            let d = server.clone();
            tokio::task::spawn_local(async move {
                if let Err(e) = serve_client(stream, d, socket_uid).await {
                    warn!(error = %e, "IPC client disconnected");
                }
            });
        }
        #[allow(unreachable_code)]
        Ok::<(), anyhow::Error>(())
    };

    tokio::select! {
        result = accept_loop => { result?; }
        _ = tokio::signal::ctrl_c() => { let _ = shutdown_tx.send(()).await; info!("panod stopped"); }
    }
    let _ = tokio::fs::remove_file(&path).await;
    Ok(())
}

async fn run_gnome_bridge(daemon: Arc<Daemon>) -> anyhow::Result<()> {
    let (bridge, mut rx) = GnomeBridge::new(64);
    let connection = zbus::connection::Builder::session()?
        .name(BUS_NAME)?
        .serve_at(OBJECT_PATH, bridge)?
        .build()
        .await?;
    info!("GNOME Shell bridge D-Bus service ready");
    while let Some(data) = rx.recv().await {
        daemon.handle_gnome_data(data).await;
    }
    drop(connection);
    Ok(())
}

async fn serve_client(
    stream: UnixStream,
    daemon: Arc<Daemon>,
    socket_uid: u32,
) -> anyhow::Result<()> {
    #[cfg(unix)]
    if stream.peer_cred()?.uid() != socket_uid {
        return Err(anyhow::anyhow!("IPC peer UID does not match daemon UID"));
    }
    let (read_half, mut write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);
    let mut line = String::new();
    let mut request_count = 0usize;
    loop {
        line.clear();
        // Bound the read itself. Checking the length after an unbounded
        // read_line would let a client allocate gigabytes before being
        // rejected, which is exactly what MAX_FRAME_BYTES exists to prevent.
        let bytes = {
            let mut limited = (&mut reader).take(MAX_FRAME_BYTES as u64 + 1);
            limited.read_line(&mut line).await?
        };
        if bytes == 0 {
            break;
        }
        if bytes > MAX_FRAME_BYTES {
            let response = Response::Failure {
                message: "IPC request exceeds the 64 KiB limit".into(),
            };
            write_half.write_all(&encode(&response)?).await?;
            break;
        }
        request_count += 1;
        if request_count > MAX_REQUESTS_PER_CONNECTION {
            let response = Response::Failure {
                message: "IPC request limit exceeded".into(),
            };
            write_half.write_all(&encode(&response)?).await?;
            break;
        }
        let response = match decode::<Request>(line.trim_end_matches(['\r', '\n']).as_bytes()) {
            Ok(request) => handle_request(request, &daemon).await,
            Err(e) => Response::Failure {
                message: format!("invalid request: {e}"),
            },
        };
        write_half.write_all(&encode(&response)?).await?;
    }
    Ok(())
}

async fn handle_request(request: Request, daemon: &Daemon) -> Response {
    let result = match request {
        Request::List(q) => daemon.query(&q.into()).map(ResponseData::Entries),
        Request::Recall { id } => daemon.recall(id).await.map(|_| ResponseData::Empty),
        Request::Pin { id, pinned } => daemon
            .set_pinned(id, pinned)
            .await
            .map(|_| ResponseData::Empty),
        Request::Delete { id } => daemon.delete(id).await.map(|_| ResponseData::Empty),
        Request::Clear => daemon.clear().await.map(ResponseData::Count),
        Request::SetPrivate { enabled } => {
            daemon.privacy().set_private_mode(enabled);
            Ok(ResponseData::Empty)
        }
        Request::Toggle => Ok(ResponseData::Empty), // GUI process owns the popup.
        Request::Status => daemon.db().count().map(|entries| {
            ResponseData::Status(StatusData {
                backend: daemon.backend().name().into(),
                entries,
                private_mode: daemon.privacy().private_mode(),
                sync_active: false,
            })
        }),
        Request::Preview { id } => daemon.load_payloads(id).map(ResponseData::Payloads),
    };
    match result {
        Ok(data) => Response::Success(data),
        Err(e) => Response::Failure {
            message: e.to_string(),
        },
    }
}

#[cfg(unix)]
fn socket_owner_uid(path: &std::path::Path) -> anyhow::Result<u32> {
    use std::os::unix::fs::MetadataExt;
    Ok(std::fs::metadata(path)?.uid())
}

#[cfg(not(unix))]
fn socket_owner_uid(_path: &std::path::Path) -> anyhow::Result<u32> {
    Ok(0)
}

fn ensure_private_directory(path: &std::path::Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn load_or_create_device_id() -> anyhow::Result<String> {
    let path = data_dir().join("device-id");
    if let Ok(value) = std::fs::read_to_string(&path) {
        let value = value.trim().to_string();
        if value.len() == 32 {
            return Ok(value);
        }
    }
    let id = blake3::hash(format!("{}:{}", std::process::id(), chrono_like_now()).as_bytes())
        .to_hex()
        .to_string()[..32]
        .to_string();
    std::fs::create_dir_all(data_dir())?;
    std::fs::write(&path, &id)?;
    set_file_permissions(&path)?;
    Ok(id)
}

fn chrono_like_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(unix)]
fn set_file_permissions(path: &std::path::Path) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(())
}
#[cfg(not(unix))]
fn set_file_permissions(_path: &std::path::Path) -> anyhow::Result<()> {
    Ok(())
}
#[cfg(unix)]
fn set_socket_permissions(path: &std::path::Path) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(())
}
#[cfg(not(unix))]
fn set_socket_permissions(_path: &std::path::Path) -> anyhow::Result<()> {
    Ok(())
}
