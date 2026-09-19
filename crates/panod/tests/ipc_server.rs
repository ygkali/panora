// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! The real IPC server on a real Unix socket, driven by the real client:
//! request round trips, the frame and request limits, the peer UID check.
//! Everything runs headless with the mock backend in a temporary directory.

#![cfg(unix)]

use panod::daemon::Daemon;
use panod::server;
use panora_core::backend::{ClipboardBackend, ClipboardEvent, MockBackend};
use panora_core::config::Config;
use panora_core::ipc::{
    client, decode, encode, QueryRequest, Request, Response, ResponseData, MAX_FRAME_BYTES,
    MAX_REQUESTS_PER_CONNECTION,
};
use panora_core::model::{ClipboardData, MimePayload, Selection};
use panora_core::storage::{BlobStore, Cipher, Database, MasterKey};
use panora_core::sync::NoopSync;
use std::os::unix::fs::MetadataExt as _;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

struct Server {
    daemon: Rc<Daemon>,
    backend: Arc<MockBackend>,
    socket: PathBuf,
    _dir: tempfile::TempDir,
}

fn build_daemon(dir: &Path) -> (Rc<Daemon>, Arc<MockBackend>) {
    let backend = Arc::new(MockBackend::new());
    let key = MasterKey::generate();
    let db = Database::open(dir.join("history.db"), Cipher::new(&key)).unwrap();
    let blobs = BlobStore::open(dir.join("blobs"), Cipher::new(&key)).unwrap();
    let daemon = Daemon::new(
        backend.clone(),
        db,
        blobs,
        Config::default(),
        Arc::new(NoopSync),
        "ipc-test-device".into(),
    );
    (Rc::new(daemon), backend)
}

/// Bind a socket in a fresh directory and start serving it on the current
/// `LocalSet`. `uid` is what the server believes the socket owner is.
fn start(uid: Option<u32>) -> Server {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("panora.sock");
    let (daemon, backend) = build_daemon(dir.path());
    let listener = UnixListener::bind(&socket).unwrap();
    let uid = uid.unwrap_or_else(|| std::fs::metadata(&socket).unwrap().uid());
    let serving = daemon.clone();
    tokio::task::spawn_local(async move {
        let _ = server::serve(listener, serving, uid).await;
    });
    Server {
        daemon,
        backend,
        socket,
        _dir: dir,
    }
}

/// Feed one clipboard change through the mock backend into the daemon.
async fn capture(server: &Server, text: &str) {
    server
        .backend
        .offer(
            Selection::Clipboard,
            ClipboardData {
                selection: Selection::Clipboard,
                payloads: vec![MimePayload::new("text/plain", text.as_bytes())],
                offered_mimes: vec!["text/plain".into()],
                source_app: Some("test".into()),
            },
        )
        .await
        .unwrap();
    server
        .daemon
        .handle_event(ClipboardEvent::changed(
            Selection::Clipboard,
            vec!["text/plain".into()],
            Some("test".into()),
        ))
        .await;
}

/// One raw JSON-lines exchange over a fresh connection. `None` when the
/// server hung up without answering: a clean EOF, or a reset when it closed
/// with our request still unread (what a refused peer sees).
async fn raw_call(socket: &Path, line: &[u8]) -> Option<Response> {
    let mut stream = UnixStream::connect(socket).await.unwrap();
    if stream.write_all(line).await.is_err() {
        return None;
    }
    let mut reader = BufReader::new(stream);
    let mut reply = String::new();
    match reader.read_line(&mut reply).await {
        Ok(0) | Err(_) => None,
        Ok(_) => Some(decode(reply.trim_end().as_bytes()).unwrap()),
    }
}

#[tokio::test]
async fn requests_round_trip_through_the_blocking_client() {
    // `client::call` resolves the socket from XDG_RUNTIME_DIR; only this
    // test touches the environment, so no other test can race it.
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let server = start(None);
            std::env::set_var("XDG_RUNTIME_DIR", server.socket.parent().unwrap());
            capture(&server, "first entry").await;
            capture(&server, "second entry").await;

            // The client blocks on a std socket, so it must not run on the
            // thread that drives the server.
            let call =
                |request: Request| tokio::task::spawn_blocking(move || client::call(&request));

            let status = match call(Request::Status).await.unwrap().unwrap() {
                ResponseData::Status(status) => status,
                other => panic!("unexpected {other:?}"),
            };
            assert_eq!(status.backend, "mock");
            assert_eq!(status.entries, 2);
            assert!(!status.private_mode);

            let entries = match call(Request::List(QueryRequest {
                search: Some("sec".into()),
                ..Default::default()
            }))
            .await
            .unwrap()
            .unwrap()
            {
                ResponseData::Entries(entries) => entries,
                other => panic!("unexpected {other:?}"),
            };
            assert_eq!(entries.len(), 1);
            assert_eq!(entries[0].preview, "second entry");
            let id = entries[0].id;

            assert!(matches!(
                call(Request::Pin { id, pinned: true })
                    .await
                    .unwrap()
                    .unwrap(),
                ResponseData::Empty
            ));
            let payloads = match call(Request::Preview { id }).await.unwrap().unwrap() {
                ResponseData::Payloads(payloads) => payloads,
                other => panic!("unexpected {other:?}"),
            };
            assert_eq!(payloads[0].data, b"second entry");

            assert!(matches!(
                call(Request::Recall {
                    id,
                    paste: false,
                    mime: None
                })
                .await
                .unwrap()
                .unwrap(),
                ResponseData::Recalled { pasted: false }
            ));
            let offered = server
                .backend
                .read(Selection::Clipboard, "text/plain")
                .await
                .unwrap();
            assert_eq!(offered, b"second entry");

            assert!(matches!(
                call(Request::SetPrivate { enabled: true })
                    .await
                    .unwrap()
                    .unwrap(),
                ResponseData::Empty
            ));
            assert!(server.daemon.private_mode());

            // Errors come back as messages, and the not-found one keeps the
            // prefix the CLI maps to its exit status.
            let missing = call(Request::Delete { id: 424_242 }).await.unwrap();
            match missing {
                Err(panora_core::Error::Ipc(message)) => {
                    assert!(message.starts_with("entry not found"), "{message}")
                }
                other => panic!("unexpected {other:?}"),
            }

            // Clear keeps the pinned entry.
            assert!(matches!(
                call(Request::Clear).await.unwrap().unwrap(),
                ResponseData::Count(1)
            ));
            assert_eq!(server.daemon.db().count().unwrap(), 1);
        })
        .await;
}

#[tokio::test]
async fn oversized_frame_is_rejected_before_parsing() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let server = start(None);
            let mut line = vec![b'x'; MAX_FRAME_BYTES + 1];
            line.push(b'\n');
            let reply = raw_call(&server.socket, &line).await.unwrap();
            match reply {
                Response::Failure { message } => assert!(message.contains("64 KiB"), "{message}"),
                other => panic!("unexpected {other:?}"),
            }
        })
        .await;
}

#[tokio::test]
async fn invalid_json_gets_a_failure_reply_and_the_connection_survives() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let server = start(None);
            let mut stream = UnixStream::connect(&server.socket).await.unwrap();
            stream.write_all(b"{not json\n").await.unwrap();
            let mut reader = BufReader::new(stream);
            let mut reply = String::new();
            reader.read_line(&mut reply).await.unwrap();
            let response: Response = decode(reply.trim_end().as_bytes()).unwrap();
            assert!(matches!(response, Response::Failure { message } if message.starts_with("invalid request")));

            // Same connection, a valid request afterwards still works.
            reader
                .get_mut()
                .write_all(&encode(&Request::Status).unwrap())
                .await
                .unwrap();
            reply.clear();
            reader.read_line(&mut reply).await.unwrap();
            let response: Response = decode(reply.trim_end().as_bytes()).unwrap();
            assert!(matches!(
                response.into_result().unwrap(),
                ResponseData::Status(_)
            ));
        })
        .await;
}

#[tokio::test]
async fn request_limit_closes_a_chatty_connection() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let server = start(None);
            let stream = UnixStream::connect(&server.socket).await.unwrap();
            let (read_half, mut write_half) = stream.into_split();
            let mut reader = BufReader::new(read_half);
            let request = encode(&Request::Status).unwrap();
            for i in 0..=MAX_REQUESTS_PER_CONNECTION {
                write_half.write_all(&request).await.unwrap();
                let mut reply = String::new();
                reader.read_line(&mut reply).await.unwrap();
                let response: Response = decode(reply.trim_end().as_bytes()).unwrap();
                if i < MAX_REQUESTS_PER_CONNECTION {
                    assert!(matches!(response, Response::Success(_)), "request {i}");
                } else {
                    assert!(
                        matches!(response, Response::Failure { message } if message.contains("limit")),
                        "request {i} must be refused"
                    );
                }
            }
            // The server hung up after the refusal.
            let mut rest = Vec::new();
            let n = reader.read_to_end(&mut rest).await.unwrap();
            assert_eq!(n, 0);
        })
        .await;
}

#[tokio::test]
async fn foreign_peer_uid_is_dropped_without_a_reply() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let mine = std::fs::metadata("/proc/self")
                .map(|m| m.uid())
                .unwrap_or(0);
            // The server is told the socket belongs to someone else, so our
            // own connection must look foreign to it.
            let server = start(Some(mine.wrapping_add(1)));
            let reply = raw_call(&server.socket, &encode(&Request::Status).unwrap()).await;
            assert!(reply.is_none(), "a foreign uid gets no bytes back");
        })
        .await;
}
