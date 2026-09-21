// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! STO-06: one IPC round trip (`Status`) over a real Unix socket, against
//! the real server with a `MockBackend` — the same client path
//! `panora-cli`/the popup use, not a shortcut through `Daemon` directly.
//! `docs/benchmark.md` records the measured numbers this produces.
//!
//! `Daemon` holds an `Rc`, so it cannot run on criterion's async/multi-
//! thread executor; the server runs on its own background OS thread with
//! its own current-thread runtime instead, and each benchmark iteration
//! does the same blocking, one-connection-per-request socket I/O
//! `panora_core::ipc::client::call` does.

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use panod::daemon::Daemon;
use panora_core::backend::MockBackend;
use panora_core::config::Config;
use panora_core::ipc::{decode, encode, Request, Response};
use panora_core::storage::{BlobStore, Cipher, Database, MasterKey};
use panora_core::sync::NoopSync;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::sync::Arc;

/// Start the real server on its own thread, bound to a socket under a
/// throwaway directory; returns the socket path (the directory is leaked
/// on purpose, harmless for a one-shot benchmark process).
fn start_server() -> std::path::PathBuf {
    let dir = tempfile::tempdir().unwrap().keep();
    let socket = dir.join("panod.sock");
    let socket_for_thread = socket.clone();
    std::thread::spawn(move || {
        let backend = Arc::new(MockBackend::new());
        let key = MasterKey::generate();
        let db = Database::open(dir.join("history.db"), Cipher::new(&key)).unwrap();
        let blobs = BlobStore::open(dir.join("blobs"), Cipher::new(&key)).unwrap();
        let daemon = std::rc::Rc::new(Daemon::new(
            backend,
            db,
            blobs,
            Config::default(),
            Arc::new(NoopSync),
            "bench-device".into(),
        ));
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let local = tokio::task::LocalSet::new();
        runtime.block_on(local.run_until(async move {
            // `UnixListener::bind` needs an active reactor, so it has to
            // happen inside the runtime, not before `block_on`.
            let listener = tokio::net::UnixListener::bind(&socket_for_thread).unwrap();
            use std::os::unix::fs::MetadataExt as _;
            let uid = std::fs::metadata(&socket_for_thread).unwrap().uid();
            let _ = panod::server::serve(listener, daemon, uid).await;
        }));
    });
    // Give the background thread a moment to bind the socket; a benchmark
    // process is short-lived enough that a fixed, generous wait is simpler
    // than a readiness channel and does not affect what is measured below.
    for _ in 0..200 {
        if socket.exists() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    socket
}

fn status_round_trip(c: &mut Criterion) {
    let socket = start_server();

    c.bench_function("IPC round trip (Status, v2)", |b| {
        b.iter(|| {
            let stream = UnixStream::connect(&socket).unwrap();
            let mut writer = &stream;
            writer
                .write_all(&encode(&Request::Status).unwrap())
                .unwrap();
            let mut line = String::new();
            BufReader::new(&stream).read_line(&mut line).unwrap();
            let response: Response = decode(line.trim_end().as_bytes()).unwrap();
            black_box(response.into_result().unwrap());
        });
    });
}

criterion_group!(benches, status_round_trip);
criterion_main!(benches);
