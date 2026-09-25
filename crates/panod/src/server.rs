// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! Process entry: backend selection, key material, storage, the GNOME bridge
//! service, periodic maintenance and the JSON-lines IPC server.

use crate::backend::select_backend;
use crate::daemon::Daemon;
use crate::dbus_api;
use crate::gnome::{self, GnomeBridge, BUS_NAME, OBJECT_PATH};
use crate::keyring::load_or_create_master_key;
use panora_core::config::{config_path, data_dir, socket_path, Config};
use panora_core::ipc::{
    decode, encode, v3, Event, Request, Response, ResponseData, MAX_FRAME_BYTES,
    MAX_REQUESTS_PER_CONNECTION, PROTOCOL_VERSION,
};
use panora_core::storage::{BlobStore, Cipher, Database};
use panora_core::sync::NoopSync;
use std::rc::Rc;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader, Interest};
use tokio::net::{UnixListener, UnixStream};
use tracing::{error, info, warn};

/// Retention policies are re-applied this often even without new copies,
/// so `max_age_days` expires entries on quiet days too.
const MAINTENANCE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(60 * 60);

/// How often `Daemon::maybe_auto_lock` is polled. `lock_after_idle_minutes`
/// is configured in minutes, so this needs to be much finer than
/// `MAINTENANCE_INTERVAL` without busy-polling.
const IDLE_LOCK_CHECK_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);

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
    harden_process();

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

    // INT-05: a public D-Bus mirror of the Unix socket, for third-party
    // integrations that would rather speak D-Bus. Talks to the socket
    // above as its own client (must be bound already, which it is by
    // this point), so it needs nothing from `daemon` directly and a
    // failure here never affects capture or the primary IPC path.
    tokio::task::spawn_local(async move {
        if let Err(e) = dbus_api::run().await {
            warn!(error = %e, "public D-Bus API (INT-05) unavailable");
        }
    });

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

    let lock_watcher = daemon.clone();
    tokio::task::spawn_local(async move {
        if let Err(e) = watch_screen_lock(lock_watcher).await {
            warn!(error = %e, "screen lock watcher unavailable; recording continues on the lock screen");
        }
    });

    if bridge_required {
        let extension_watcher = daemon.clone();
        tokio::task::spawn_local(async move {
            if let Err(e) = watch_shell_extension(extension_watcher).await {
                warn!(error = %e, "Shell extension watcher unavailable; status cannot report a missing extension");
            }
        });
    }

    let maintenance = daemon.clone();
    tokio::task::spawn_local(async move {
        let mut ticker = tokio::time::interval(MAINTENANCE_INTERVAL);
        ticker.tick().await;
        loop {
            ticker.tick().await;
            if let Err(e) = maintenance.maintain() {
                warn!(error = %e, "periodic maintenance failed");
            }
        }
    });

    // SEC-02: `lock_after_idle_minutes` is typically minutes, not hours, so
    // it gets its own, much finer ticker rather than piggybacking on
    // `MAINTENANCE_INTERVAL`. A no-op check (`maybe_auto_lock` bails out
    // immediately) whenever idle locking is off or already engaged.
    let idle_lock = daemon.clone();
    tokio::task::spawn_local(async move {
        let mut ticker = tokio::time::interval(IDLE_LOCK_CHECK_INTERVAL);
        loop {
            ticker.tick().await;
            idle_lock.maybe_auto_lock();
        }
    });

    let outcome = tokio::select! {
        result = serve(listener, daemon.clone(), socket_uid) => result,
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

/// The master key and decrypted payloads live in this process; a core dump
/// or a ptrace from another process of the same user would hand them over.
/// `PR_SET_DUMPABLE = 0` refuses both (ADR 0003); the unit adds
/// `LimitCORE=0` on top.
fn harden_process() {
    use rustix::process::{set_dumpable_behavior, DumpableBehavior};
    if let Err(e) = set_dumpable_behavior(DumpableBehavior::NotDumpable) {
        warn!(error = %e, "could not mark the process non-dumpable");
    }
}

/// Pause recording while the session is locked. GNOME emits
/// `org.gnome.ScreenSaver.ActiveChanged(b)`, KDE and others the
/// `org.freedesktop.ScreenSaver` twin; both are matched by interface and
/// member only, so the object path each desktop picks does not matter.
async fn watch_screen_lock(daemon: Rc<Daemon>) -> zbus::Result<()> {
    use futures::StreamExt as _;
    let connection = zbus::Connection::session().await?;
    let bus = zbus::fdo::DBusProxy::new(&connection).await?;
    for interface in ["org.gnome.ScreenSaver", "org.freedesktop.ScreenSaver"] {
        let rule = zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .interface(interface)?
            .member("ActiveChanged")?
            .build();
        bus.add_match_rule(rule).await?;
    }
    let mut stream = zbus::MessageStream::from(&connection);
    while let Some(message) = stream.next().await {
        let message = message?;
        let header = message.header();
        let is_lock_signal = header.message_type() == zbus::message::Type::Signal
            && header
                .member()
                .is_some_and(|m| m.as_str() == "ActiveChanged")
            && header
                .interface()
                .is_some_and(|i| i.as_str().ends_with(".ScreenSaver"));
        if !is_lock_signal {
            continue;
        }
        match message.body().deserialize::<bool>() {
            Ok(active) => daemon.set_locked(active),
            Err(e) => warn!(error = %e, "unexpected ActiveChanged body"),
        }
    }
    Ok(())
}

/// Follow the Shell extension's bus name so `Status` can say when a GNOME
/// Wayland session has nothing feeding the daemon: an initial `NameHasOwner`,
/// then every `NameOwnerChanged` for that name.
async fn watch_shell_extension(daemon: Rc<Daemon>) -> zbus::Result<()> {
    use futures::StreamExt as _;
    let connection = zbus::Connection::session().await?;
    let bus = zbus::fdo::DBusProxy::new(&connection).await?;
    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender("org.freedesktop.DBus")?
        .interface("org.freedesktop.DBus")?
        .member("NameOwnerChanged")?
        .arg(0, gnome::SHELL_BUS_NAME)?
        .build();
    bus.add_match_rule(rule).await?;
    let name = zbus::names::BusName::try_from(gnome::SHELL_BUS_NAME)?;
    daemon.set_extension_present(bus.name_has_owner(name).await?);
    let mut stream = zbus::MessageStream::from(&connection);
    while let Some(message) = stream.next().await {
        let message = message?;
        let header = message.header();
        let is_owner_signal = header.message_type() == zbus::message::Type::Signal
            && header
                .member()
                .is_some_and(|m| m.as_str() == "NameOwnerChanged");
        if !is_owner_signal {
            continue;
        }
        match message.body().deserialize::<(String, String, String)>() {
            Ok((name, _, new_owner)) if name == gnome::SHELL_BUS_NAME => {
                daemon.set_extension_present(!new_owner.is_empty());
            }
            Ok(_) => {}
            Err(e) => warn!(error = %e, "unexpected NameOwnerChanged body"),
        }
    }
    Ok(())
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
    while let Some((data, window_title)) = rx.recv().await {
        daemon.handle_gnome_data(data, window_title).await;
    }
    drop(connection);
    Ok(())
}

/// Accept IPC clients until the listener fails; every connection is served
/// on its own local task. Public so the integration tests can run the real
/// server against a socket of their own.
pub async fn serve(
    listener: UnixListener,
    daemon: Rc<Daemon>,
    socket_uid: u32,
) -> anyhow::Result<()> {
    loop {
        let (stream, _) = listener.accept().await?;
        let d = daemon.clone();
        tokio::task::spawn_local(async move {
            if let Err(e) = dispatch_client(stream, d, socket_uid).await {
                warn!(error = %e, "IPC client disconnected");
            }
        });
    }
}

/// Peek the connection's first byte to tell a v3 client (opens with
/// [`v3::MAGIC`]) from a v2 one (opens with a bare JSON request, always
/// starting with `{`), then hand it to the matching handler. v2 clients pay
/// nothing extra: the peek doesn't consume anything, so `serve_client`'s own
/// read sees the same first byte it always has.
async fn dispatch_client(
    stream: UnixStream,
    daemon: Rc<Daemon>,
    socket_uid: u32,
) -> anyhow::Result<()> {
    loop {
        stream.readable().await?;
        match stream.try_io(Interest::READABLE, || v3::peek_first_byte(&stream)) {
            Ok(first) => {
                return match first {
                    Some(b) if b == v3::MAGIC[0] => {
                        serve_client_v3(stream, daemon, socket_uid).await
                    }
                    _ => serve_client(stream, daemon, socket_uid).await,
                };
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
            Err(e) => return Err(e.into()),
        }
    }
}

/// Serve one connection: peer UID check, then bounded JSON-lines requests.
pub async fn serve_client(
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
/// Reply to `Preview`/`Recall` while the second-layer lock (SEC-02) is
/// engaged.
fn locked_error() -> panora_core::error::Error {
    panora_core::error::Error::Ipc("history is locked; unlock with: panora-cli unlock".into())
}

pub async fn handle_request(request: Request, daemon: &Daemon) -> Response {
    // Health checks, protocol negotiation and the long-lived Subscribe
    // stream (which never reaches this function at all) should not by
    // themselves keep an idle lock from engaging.
    if !matches!(request, Request::Status | Request::Hello { .. }) {
        daemon.touch_activity();
    }
    let result = match request {
        Request::List(q) => {
            if daemon.is_app_locked() {
                daemon
                    .db()
                    .count()
                    .map(|n| ResponseData::Count(n.max(0) as usize))
            } else {
                daemon.query(&q.into()).map(ResponseData::Entries)
            }
        }
        Request::Recall {
            id,
            paste,
            mime,
            to,
        } => {
            if daemon.is_app_locked() {
                Err(locked_error())
            } else {
                daemon
                    .recall(id, paste, mime.as_deref(), to)
                    .await
                    .map(|outcome| ResponseData::Recalled {
                        pasted: outcome.pasted,
                    })
            }
        }
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
        Request::Stats => daemon.stats().map(ResponseData::Stats),
        Request::Preview { id, thumbnail } => {
            if daemon.is_app_locked() {
                Err(locked_error())
            } else if thumbnail {
                daemon
                    .thumbnail_or_full(id)
                    .await
                    .map(ResponseData::Payloads)
            } else {
                daemon.load_payloads(id).map(ResponseData::Payloads)
            }
        }
        Request::ReloadConfig => daemon.reload_config().map(|_| ResponseData::Empty),
        Request::RotateKey => daemon.rotate_key().await.map(|_| ResponseData::Empty),
        Request::Lock => daemon.engage_lock().map(|_| ResponseData::Empty),
        Request::Unlock { password } => daemon.unlock(&password).map(|_| ResponseData::Empty),
        Request::SetLockPassword {
            new_password,
            current_password,
        } => daemon
            .set_lock_password(new_password.as_deref(), current_password.as_deref())
            .await
            .map(|_| ResponseData::Empty),
        Request::Wipe => daemon.wipe().await.map(|_| ResponseData::Empty),
        Request::Export { passphrase } => {
            if daemon.is_app_locked() {
                Err(locked_error())
            } else {
                daemon.export(&passphrase).map(ResponseData::Archive)
            }
        }
        Request::Import {
            passphrase,
            archive,
        } => daemon
            .import(&passphrase, &archive)
            .await
            .map(ResponseData::Count),
        Request::Restore { id } => daemon.restore(id).await.map(|_| ResponseData::Empty),
        Request::Store {
            payloads,
            source_app,
            copy,
        } => {
            let data = panora_core::model::ClipboardData {
                selection: panora_core::model::Selection::Clipboard,
                offered_mimes: payloads.iter().map(|p| p.mime.clone()).collect(),
                payloads,
                source_app,
            };
            daemon
                .store_external(data, copy)
                .await
                .map(|entry| ResponseData::Entries(vec![entry]))
        }
        Request::Hello { max_protocol } => Ok(ResponseData::Hello {
            protocol: max_protocol.min(PROTOCOL_VERSION),
        }),
        // Only meaningful on a v3 connection, where `serve_client_v3`
        // intercepts it before it ever reaches here (it becomes an
        // event stream, not a single reply). A v2 client cannot
        // subscribe at all.
        Request::Subscribe => Err(panora_core::error::Error::Ipc(
            "Subscribe requires the v3 protocol".into(),
        )),
    };
    match result {
        Ok(data) => Response::Success(data),
        Err(e) => Response::Failure {
            message: e.to_string(),
        },
    }
}

/// Serve one v3 connection: peer UID check (same rule as v2), consume the
/// magic preamble, then bounded binary-framed requests — until either the
/// peer disconnects or a `Subscribe` turns the rest of the connection into
/// an event stream.
async fn serve_client_v3(
    mut stream: UnixStream,
    daemon: Rc<Daemon>,
    socket_uid: u32,
) -> anyhow::Result<()> {
    if stream.peer_cred()?.uid() != socket_uid {
        return Err(anyhow::anyhow!("IPC peer UID does not match daemon UID"));
    }
    // The magic itself carries no fds and is short enough that a plain read
    // is fine; framed requests after it always go through `read_v3_frame`.
    let mut magic = [0u8; 4];
    stream.read_exact(&mut magic).await?;
    if magic != *v3::MAGIC {
        return Err(anyhow::anyhow!(
            "v3 connection did not open with the expected magic"
        ));
    }

    let mut request_count = 0usize;
    let mut reader = v3::server::FrameReader::default();
    loop {
        let Some((header, inline, fds)) = read_v3_frame(&stream, &mut reader).await? else {
            return Ok(()); // peer closed
        };
        reader = v3::server::FrameReader::default();
        request_count += 1;
        if request_count > MAX_REQUESTS_PER_CONNECTION {
            let frame = v3::server::encode_response(Response::Failure {
                message: "IPC request limit exceeded".into(),
            })?;
            write_v3_frame(&stream, frame).await?;
            return Ok(());
        }
        let request: Request = match v3::server::decode(&header, &inline, fds) {
            Ok(r) => r,
            Err(e) => {
                let frame = v3::server::encode_response(Response::Failure {
                    message: format!("invalid request: {e}"),
                })?;
                write_v3_frame(&stream, frame).await?;
                continue;
            }
        };
        match request {
            Request::Subscribe => return subscribe_loop(&stream, &daemon).await,
            other => {
                let response = handle_request(other, &daemon).await;
                let frame = v3::server::encode_response(response)?;
                write_v3_frame(&stream, frame).await?;
            }
        }
    }
}

/// Read one v3 frame, awaiting readability between non-blocking attempts.
/// `Ok(None)` means the peer closed the connection.
async fn read_v3_frame(
    stream: &UnixStream,
    reader: &mut v3::server::FrameReader,
) -> anyhow::Result<Option<(Vec<u8>, Vec<u8>, Vec<std::os::fd::OwnedFd>)>> {
    loop {
        stream.readable().await?;
        match stream.try_io(Interest::READABLE, || reader.poll_once(stream)) {
            Ok(v3::server::FramePoll::Pending) => continue,
            Ok(v3::server::FramePoll::Closed) => return Ok(None),
            Ok(v3::server::FramePoll::Ready {
                header,
                inline,
                fds,
            }) => return Ok(Some((header, inline, fds))),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
            Err(e) => return Err(e.into()),
        }
    }
}

/// Write one v3 frame, awaiting writability between non-blocking attempts.
async fn write_v3_frame(stream: &UnixStream, frame: v3::server::WireFrame) -> anyhow::Result<()> {
    let mut writer = v3::server::FrameWriter::new(frame);
    while !writer.is_done() {
        stream.writable().await?;
        match stream.try_io(Interest::WRITABLE, || writer.poll_once(stream)) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

/// A `Subscribe` connection: no more requests are read from it. It gets an
/// immediate `Event::Changed` with the current revision, then one more
/// every time `revision` changes, until the client disconnects (detected by
/// the socket becoming readable, which for a connection that never sends
/// anything else only happens on EOF or a protocol violation — either way
/// the subscription ends).
async fn subscribe_loop(stream: &UnixStream, daemon: &Rc<Daemon>) -> anyhow::Result<()> {
    let mut revisions = daemon.watch_revision();
    let mut revision = *revisions.borrow();
    loop {
        let frame = v3::server::encode_event(&Event::Changed { revision })?;
        write_v3_frame(stream, frame).await?;
        // Wait for the next *real* change. `stream.readable()` can resolve
        // on stale/edge-triggered readiness left over from reading the
        // `Subscribe` request itself, with nothing actually there yet
        // (`peek_first_byte` then reports `WouldBlock`) — that must only
        // re-poll this inner wait, never re-send the frame above, or the
        // client sees the same revision twice.
        loop {
            tokio::select! {
                changed = revisions.changed() => {
                    if changed.is_err() {
                        return Ok(()); // daemon shutting down
                    }
                    revision = *revisions.borrow();
                    break;
                }
                ready = stream.readable() => {
                    ready?;
                    match stream.try_io(Interest::READABLE, || v3::peek_first_byte(stream)) {
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
                        _ => return Ok(()), // EOF, or a request this connection no longer accepts
                    }
                }
            }
        }
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
