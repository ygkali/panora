// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! Process entry: backend selection, key material, storage, the GNOME bridge
//! service, periodic maintenance and the JSON-lines IPC server.

use crate::backend::select_backend;
use crate::daemon::Daemon;
use crate::gnome::{self, GnomeBridge, BUS_NAME, OBJECT_PATH};
use crate::keyring::load_or_create_master_key;
use panora_core::config::{config_path, data_dir, socket_path, Config};
use panora_core::ipc::{
    decode, encode, Request, Response, ResponseData, MAX_FRAME_BYTES, MAX_REQUESTS_PER_CONNECTION,
};
use panora_core::storage::{BlobStore, Cipher, Database};
use panora_core::sync::NoopSync;
use std::rc::Rc;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tracing::{error, info, warn};

/// Retention policies are re-applied this often even without new copies,
/// so `max_age_days` expires entries on quiet days too.
const MAINTENANCE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(60 * 60);

/// Blocking entry point used by `main`.
pub fn main() -> anyhow::Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let local = tokio::task::LocalSet::new();
    runtime.block_on(local.run_until(run_app()))
}

async fn run_app() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "info".into()))
        .compact()
        .init();

    let config =
        Config::load().map_err(|e| anyhow::anyhow!("config {}: {e}", config_path().display()))?;
    let backend = select_backend().await.map_err(|e| anyhow::anyhow!("{e}"))?;
    info!(backend = backend.name(), "clipboard backend selected");

    let data_root = data_dir();
    ensure_private_directory(&data_root)?;
    let master = load_or_create_master_key()
        .await
        .map_err(|e| anyhow::anyhow!("keyring unavailable: {e}"))?;
    let db = Database::open(data_root.join("history.db"), Cipher::new(&master))?;
    let blobs = BlobStore::open(data_root.join("blobs"), Cipher::new(&master))?;
    let device_id = load_or_create_device_id()?;
    let daemon = Rc::new(Daemon::new(
        backend,
        db,
        blobs,
        config,
        Arc::new(NoopSync),
        device_id,
    ));
    if let Err(e) = daemon.collect_garbage() {
        warn!(error = %e, "startup maintenance failed");
    }

    // A capture loop that dies (display connection lost, compositor gone)
    // must take the process down so systemd restarts it, instead of leaving
    // an IPC server that silently records nothing. The same goes for the
    // GNOME bridge service when capture depends on it.
    let (fatal_tx, fatal_rx) = tokio::sync::mpsc::channel::<String>(2);

    let bridge_daemon = daemon.clone();
    let bridge_fatal = fatal_tx.clone();
    let bridge_required = daemon.backend().capabilities().needs_bridge;
    tokio::task::spawn_local(async move {
        if let Err(e) = run_gnome_bridge(bridge_daemon).await {
            if bridge_required {
                error!(error = %e, "GNOME bridge service failed and capture depends on it");
                let _ = bridge_fatal.send(format!("GNOME bridge: {e}")).await;
            } else {
                warn!(error = %e, "GNOME bridge unavailable; install/enable the extension for GNOME Wayland capture");
            }
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
    let capture_fatal = fatal_tx.clone();
    tokio::task::spawn_local(async move {
        if let Err(e) = capture.run(shutdown_rx).await {
            error!(error = %e, "capture loop stopped");
            let _ = capture_fatal.send(e.to_string()).await;
        }
    });
    drop(fatal_tx);
    let mut fatal_rx = fatal_rx;

    let maintenance = daemon.clone();
    tokio::task::spawn_local(async move {
        let mut ticker = tokio::time::interval(MAINTENANCE_INTERVAL);
        ticker.tick().await;
        loop {
            ticker.tick().await;
            if let Err(e) = maintenance.collect_garbage() {
                warn!(error = %e, "periodic maintenance failed");
            }
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

    let outcome = tokio::select! {
        result = accept_loop => result,
        _ = shutdown_signal() => {
            let _ = shutdown_tx.send(()).await;
            info!("panod stopped");
            Ok(())
        }
        reason = fatal_rx.recv() => Err(anyhow::anyhow!(
            "capture stopped: {}",
            reason.unwrap_or_else(|| "capture task ended".into())
        )),
    };
    let _ = tokio::fs::remove_file(&path).await;
    outcome
}

/// SIGINT (terminal) or SIGTERM (systemd stop).
async fn shutdown_signal() {
    use tokio::signal::unix::{signal, SignalKind};
    let mut term = match signal(SignalKind::terminate()) {
        Ok(term) => term,
        Err(e) => {
            warn!(error = %e, "cannot listen for SIGTERM");
            let _ = tokio::signal::ctrl_c().await;
            return;
        }
    };
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = term.recv() => {}
    }
}

async fn run_gnome_bridge(daemon: Rc<Daemon>) -> anyhow::Result<()> {
    let (bridge, mut rx) = GnomeBridge::new(64, daemon.backend().capabilities().needs_bridge);
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
    daemon: Rc<Daemon>,
    socket_uid: u32,
) -> anyhow::Result<()> {
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

/// Dispatch one request. Every arm maps to a daemon method; errors become
/// `Response::Failure` with a message safe to show to the user.
pub async fn handle_request(request: Request, daemon: &Daemon) -> Response {
    let result =
        match request {
            Request::List(q) => daemon.query(&q.into()).map(ResponseData::Entries),
            Request::Recall { id, paste, mime } => daemon
                .recall(id, paste, mime.as_deref())
                .await
                .map(|outcome| ResponseData::Recalled {
                    pasted: outcome.pasted,
                }),
            Request::Pin { id, pinned } => daemon
                .set_pinned(id, pinned)
                .await
                .map(|_| ResponseData::Empty),
            Request::Delete { id } => daemon.delete(id).await.map(|_| ResponseData::Empty),
            Request::Clear => daemon.clear().await.map(ResponseData::Count),
            Request::SetPrivate { enabled } => {
                daemon.set_private_mode(enabled);
                Ok(ResponseData::Empty)
            }
            Request::Toggle => gnome::activate_gui().await.map(|_| ResponseData::Empty),
            Request::Status => daemon.status().map(ResponseData::Status),
            Request::Preview { id } => daemon.load_payloads(id).map(ResponseData::Payloads),
            Request::ReloadConfig => daemon.reload_config().map(|_| ResponseData::Empty),
        };
    match result {
        Ok(data) => Response::Success(data),
        Err(e) => Response::Failure {
            message: e.to_string(),
        },
    }
}

fn socket_owner_uid(path: &std::path::Path) -> anyhow::Result<u32> {
    use std::os::unix::fs::MetadataExt;
    Ok(std::fs::metadata(path)?.uid())
}

fn ensure_private_directory(path: &std::path::Path) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    Ok(())
}

fn load_or_create_device_id() -> anyhow::Result<String> {
    let path = data_dir().join("device-id");
    if let Ok(value) = std::fs::read_to_string(&path) {
        let value = value.trim().to_string();
        if value.len() == 32 && value.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Ok(value);
        }
    }
    let id = blake3::hash(
        format!(
            "{}:{}:{}",
            std::process::id(),
            crate::daemon::unix_now(),
            data_dir().display()
        )
        .as_bytes(),
    )
    .to_hex()
    .to_string()[..32]
        .to_string();
    std::fs::create_dir_all(data_dir())?;
    std::fs::write(&path, &id)?;
    set_file_permissions(&path)?;
    Ok(id)
}

fn set_file_permissions(path: &std::path::Path) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(())
}

fn set_socket_permissions(path: &std::path::Path) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(())
}
