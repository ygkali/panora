// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! End-to-end checks of the native X11 backend against a real X server.
//!
//! Every test returns early without `DISPLAY` so `cargo test` stays green on
//! headless machines; CI runs them under Xvfb (see `.github/workflows/ci.yml`).

#![cfg(unix)]

use panod::backend::x11::X11Backend;
use panora_core::backend::{ClipboardBackend, EventKind};
use panora_core::model::{ClipboardData, MimePayload, Selection};
use std::process::{Command, Stdio};
use std::time::Duration;

fn backend() -> Option<X11Backend> {
    if std::env::var_os("DISPLAY").is_none() {
        eprintln!("DISPLAY not set; skipping X11 integration test");
        return None;
    }
    Some(X11Backend::connect().expect("X server with XFIXES"))
}

fn data(payloads: Vec<MimePayload>) -> ClipboardData {
    ClipboardData {
        selection: Selection::Clipboard,
        offered_mimes: payloads.iter().map(|p| p.mime.clone()).collect(),
        payloads,
        source_app: Some("test".into()),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn offer_serves_every_format_and_text_aliases() {
    let Some(backend) = backend() else { return };
    backend
        .offer(
            Selection::Clipboard,
            data(vec![
                MimePayload::new("text/plain;charset=utf-8", "merhaba dünya"),
                MimePayload::new("text/html", "<b>merhaba</b> dünya"),
            ]),
        )
        .await
        .unwrap();

    let targets = backend.read_targets(Selection::Clipboard).await.unwrap();
    for expected in ["TARGETS", "TIMESTAMP", "text/html", "UTF8_STRING", "STRING"] {
        assert!(
            targets.iter().any(|t| t == expected),
            "missing {expected} in {targets:?}"
        );
    }
    assert_eq!(
        backend
            .read(Selection::Clipboard, "text/html")
            .await
            .unwrap(),
        "<b>merhaba</b> dünya".as_bytes()
    );
    assert_eq!(
        backend
            .read(Selection::Clipboard, "UTF8_STRING")
            .await
            .unwrap(),
        "merhaba dünya".as_bytes()
    );
    assert_eq!(
        backend
            .read(Selection::Clipboard, "text/plain")
            .await
            .unwrap(),
        "merhaba dünya".as_bytes()
    );
    assert!(backend
        .read(Selection::Clipboard, "image/png")
        .await
        .is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn large_payload_roundtrips_through_incr() {
    let Some(backend) = backend() else { return };
    let big: Vec<u8> = (0..3 * 1024 * 1024).map(|i| (i % 251) as u8).collect();
    backend
        .offer(
            Selection::Clipboard,
            data(vec![MimePayload::new(
                "application/octet-stream",
                big.clone(),
            )]),
        )
        .await
        .unwrap();
    let back = backend
        .read(Selection::Clipboard, "application/octet-stream")
        .await
        .unwrap();
    assert_eq!(back.len(), big.len());
    assert!(back == big, "INCR transfer corrupted the payload");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn watch_reports_other_owners_but_not_our_own_offers() {
    let Some(backend) = backend() else { return };
    let mut rx = backend.watch(Selection::Clipboard).await.unwrap();

    // Our own offer must not come back as a capture event.
    backend
        .offer(
            Selection::Clipboard,
            data(vec![MimePayload::new("text/plain", "own offer")]),
        )
        .await
        .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(600), rx.recv())
            .await
            .is_err(),
        "watcher must ignore panod's own selection ownership"
    );

    // A foreign owner (xclip) must produce a Changed event with its TARGETS.
    if Command::new("xclip").arg("-version").output().is_err() {
        eprintln!("xclip not installed; skipping foreign-owner check");
        return;
    }
    let mut child = Command::new("xclip")
        .args(["-selection", "clipboard", "-in", "-t", "text/plain"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    {
        use std::io::Write;
        child
            .stdin
            .take()
            .unwrap()
            .write_all(b"from xclip")
            .unwrap();
    }
    let event = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("change event within 5s")
        .expect("watcher alive");
    assert_eq!(event.kind, EventKind::Changed);
    assert!(
        event.offered_mimes.iter().any(|m| m == "text/plain"),
        "TARGETS should list text/plain: {:?}",
        event.offered_mimes
    );
    assert_eq!(
        backend
            .read(Selection::Clipboard, "text/plain")
            .await
            .unwrap(),
        b"from xclip"
    );
    let _ = child.wait();
}
