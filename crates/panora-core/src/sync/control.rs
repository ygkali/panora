// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! The control protocol of the `panora-sync` service: how its command line
//! and the GUI ask it to pair, list or remove devices.
//!
//! JSON lines over a 0600 Unix socket in the user's runtime directory. A
//! client sends one [`Request`](crate::sync::control::Request), then reads
//! [`Event`](crate::sync::control::Event)s until `done` or
//! `error`. Pairing questions arrive as `ask`, answered with an `answer`
//! request on the same connection; closing the connection cancels what is
//! in progress.
//!
//! Only the message types and a small blocking client live here, so the
//! GUI (in the main package, which has no network code) can talk to the
//! service without linking any of it. Results and failures are carried as
//! kinds, not sentences: each front end words them in its own language.

use crate::i18n::{fill, Strings};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Longest control line either side accepts.
pub const MAX_LINE: u64 = 64 * 1024;

/// Where the service listens.
pub fn socket_path() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(crate::config::data_dir)
        .join("panora-sync.sock")
}

/// What a client asks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    /// Remove a device: its full fingerprint, its name, or at least four
    /// characters of its fingerprint.
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

/// One device of the group, as `status` lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceRow {
    /// Device name.
    pub name: String,
    /// Identity fingerprint.
    pub fingerprint: String,
    /// This device.
    pub this_device: bool,
    /// A session with it is up.
    pub connected: bool,
}

/// This device, its group and its connections.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Status {
    /// This device's name.
    pub device_name: String,
    /// This device's identity fingerprint.
    pub fingerprint: String,
    /// Where the service listens.
    pub listen: String,
    /// Whether a group exists at all.
    pub in_group: bool,
    /// Whether this device is a member of it right now.
    pub member: bool,
    /// Whether this device holds the current group key.
    pub has_key: bool,
    /// Roster epoch.
    pub epoch: u64,
    /// The device list.
    pub devices: Vec<DeviceRow>,
}

impl Status {
    /// Whether this device may invite others: it is a member of its group,
    /// or has none yet (inviting creates one).
    pub fn can_invite(&self) -> bool {
        !self.in_group || self.member
    }

    /// Whether this device may join a group: it is not a member of one.
    pub fn can_join(&self) -> bool {
        !(self.in_group && self.member)
    }
}

/// How a request ended well.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Outcome {
    /// A device joined this device's group (`invite`, `pair`).
    DeviceJoined {
        /// Its name.
        name: String,
        /// Its fingerprint.
        fingerprint: String,
    },
    /// This device joined a group (`join`, `join_code`).
    JoinedGroup,
    /// A device was removed and the group key replaced.
    Removed {
        /// Its name.
        name: String,
        /// Its fingerprint.
        fingerprint: String,
    },
    /// This device left its group.
    Left,
}

/// Why a request, or one pairing attempt, failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Failure {
    /// Cancelled on this device.
    Cancelled,
    /// The other device said no, or the codes did not match.
    Rejected,
    /// The invitation or the pairing window ran out.
    Expired,
    /// Pairing is not open on the other device.
    NotOpen,
    /// No device to pair with was found, or it did not answer.
    Unreachable,
    /// Several devices wait for a code; the address of one is needed.
    Ambiguous,
    /// The link is not a Panora invitation, or is damaged.
    InvalidLink,
    /// The address given is not `ip:port`.
    InvalidAddress,
    /// One device showed an invitation, the other waits for a code.
    WrongMode,
    /// The group cannot take the device (already in it, or full).
    Refused,
    /// A cryptographic check failed: not the device it claims to be.
    Verification,
    /// Anything else; the message says what.
    Other,
}

impl Failure {
    /// The failure in the user's language. The service's English
    /// `message` appears only for [`Failure::Other`], the kind that has no
    /// sentence of its own.
    pub fn describe(self, s: &Strings, message: &str) -> String {
        match self {
            Failure::Cancelled => s.sync_failure_cancelled.into(),
            Failure::Rejected => s.sync_failure_rejected.into(),
            Failure::Expired => s.sync_failure_expired.into(),
            Failure::NotOpen => s.sync_failure_not_open.into(),
            Failure::Unreachable => s.sync_failure_unreachable.into(),
            Failure::Ambiguous => s.sync_failure_ambiguous.into(),
            Failure::InvalidLink => s.sync_failure_invalid_link.into(),
            Failure::InvalidAddress => s.sync_failure_invalid_address.into(),
            Failure::WrongMode => s.sync_failure_wrong_mode.into(),
            Failure::Refused => s.sync_failure_refused.into(),
            Failure::Verification => s.sync_failure_verification.into(),
            Failure::Other => fill(s.sync_failure_other, "e", message),
        }
    }
}

/// What the service reports.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    /// Reply to `status`.
    Status {
        /// The report.
        status: Status,
    },
    /// The invitation link to show, as text and as a QR code.
    Link {
        /// `panora-pair:1?...`
        link: String,
        /// Unix time it stops working.
        expires_at: i64,
    },
    /// A code window is open; waiting for a device to join.
    Listening {
        /// Unix time the window closes.
        expires_at: i64,
    },
    /// Joining the group of the device with this fingerprint.
    Joining {
        /// The inviting device.
        fingerprint: String,
    },
    /// One attempt failed; the window stays open for another.
    AttemptFailed {
        /// Why.
        failure: Failure,
        /// Details, in English.
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
        outcome: Outcome,
    },
    /// Failed.
    Error {
        /// Why.
        failure: Failure,
        /// Details, in English.
        message: String,
    },
}

impl Event {
    /// Whether this is the last event of a request.
    pub fn is_final(&self) -> bool {
        matches!(
            self,
            Event::Status { .. } | Event::Done { .. } | Event::Error { .. }
        )
    }
}

/// A blocking connection to the service, for front ends without an async
/// runtime (the GUI runs it on a worker thread).
#[cfg(unix)]
pub struct Client {
    reader: std::io::BufReader<std::os::unix::net::UnixStream>,
    writer: std::os::unix::net::UnixStream,
}

#[cfg(unix)]
impl Client {
    /// Connect to the running service; an error means it is not running.
    pub fn connect(path: &std::path::Path) -> std::io::Result<Self> {
        let stream = std::os::unix::net::UnixStream::connect(path)?;
        let writer = stream.try_clone()?;
        Ok(Self {
            reader: std::io::BufReader::new(stream),
            writer,
        })
    }

    /// Give up on a read or a write after `timeout` (`None`: wait forever,
    /// the default; right for pairing, which waits on a person).
    pub fn set_timeout(&self, timeout: Option<std::time::Duration>) -> std::io::Result<()> {
        self.reader.get_ref().set_read_timeout(timeout)?;
        self.writer.set_write_timeout(timeout)
    }

    /// A handle that sends answers and can cancel the request from another
    /// thread while this one waits in [`Client::next_event`].
    pub fn handle(&self) -> std::io::Result<Handle> {
        Ok(Handle(self.writer.try_clone()?))
    }

    /// Send a request (or an answer).
    pub fn send(&mut self, request: &Request) -> std::io::Result<()> {
        write_line(&mut self.writer, request)
    }

    /// The next event; `None` when the service closed the connection.
    pub fn next_event(&mut self) -> std::io::Result<Option<Event>> {
        use std::io::{BufRead, Read};
        let mut line = Vec::new();
        let n = (&mut self.reader)
            .take(MAX_LINE)
            .read_until(b'\n', &mut line)?;
        if n == 0 {
            return Ok(None);
        }
        if line.last() != Some(&b'\n') {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "control line too long",
            ));
        }
        serde_json::from_slice(&line)
            .map(Some)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    }
}

/// The writing side of a [`Client`], shareable with another thread.
#[cfg(unix)]
pub struct Handle(std::os::unix::net::UnixStream);

#[cfg(unix)]
impl Handle {
    /// Answer the last `ask`.
    pub fn answer(&mut self, accept: bool) -> std::io::Result<()> {
        write_line(&mut self.0, &Request::Answer { accept })
    }

    /// Close the connection: the service cancels what is in progress and
    /// the reading side sees the end of the stream.
    pub fn cancel(&self) {
        let _ = self.0.shutdown(std::net::Shutdown::Both);
    }
}

#[cfg(unix)]
fn write_line(writer: &mut impl std::io::Write, request: &Request) -> std::io::Result<()> {
    let mut line = serde_json::to_vec(request)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    line.push(b'\n');
    writer.write_all(&line)?;
    writer.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_and_events_keep_their_wire_names() {
        assert_eq!(
            serde_json::to_string(&Request::JoinCode { address: None }).unwrap(),
            r#"{"cmd":"join_code","address":null}"#
        );
        let done = Event::Done {
            outcome: Outcome::DeviceJoined {
                name: "laptop".into(),
                fingerprint: "ab".into(),
            },
        };
        assert_eq!(
            serde_json::to_string(&done).unwrap(),
            r#"{"event":"done","outcome":{"kind":"device_joined","name":"laptop","fingerprint":"ab"}}"#
        );
        let error = Event::Error {
            failure: Failure::NotOpen,
            message: "x".into(),
        };
        let text = serde_json::to_string(&error).unwrap();
        assert!(text.contains(r#""failure":"not_open""#), "{text}");
        assert_eq!(serde_json::from_str::<Event>(&text).unwrap(), error);
    }

    #[test]
    fn every_failure_has_its_own_sentence_in_both_languages() {
        use crate::i18n::Language;
        let all = [
            Failure::Cancelled,
            Failure::Rejected,
            Failure::Expired,
            Failure::NotOpen,
            Failure::Unreachable,
            Failure::Ambiguous,
            Failure::InvalidLink,
            Failure::InvalidAddress,
            Failure::WrongMode,
            Failure::Refused,
            Failure::Verification,
        ];
        for language in [Language::English, Language::Turkish] {
            let s = language.strings();
            let texts: std::collections::HashSet<String> =
                all.iter().map(|f| f.describe(s, "detail")).collect();
            assert_eq!(texts.len(), all.len());
            assert!(texts.iter().all(|t| !t.contains("detail")));
            assert!(Failure::Other.describe(s, "detail").contains("detail"));
        }
    }

    #[test]
    fn only_the_last_event_of_a_request_is_final() {
        assert!(!Event::Listening { expires_at: 1 }.is_final());
        assert!(!Event::Code { code: "1".into() }.is_final());
        assert!(Event::Done {
            outcome: Outcome::Left
        }
        .is_final());
    }

    #[test]
    fn who_may_invite_and_who_may_join() {
        let mut status = Status {
            device_name: "d".into(),
            fingerprint: "f".into(),
            listen: "[::]:47100".into(),
            in_group: false,
            member: false,
            has_key: false,
            epoch: 0,
            devices: Vec::new(),
        };
        assert!(status.can_invite() && status.can_join());
        status.in_group = true;
        status.member = true;
        assert!(status.can_invite() && !status.can_join());
        // Removed from the group: can only join again (or leave).
        status.member = false;
        assert!(!status.can_invite() && status.can_join());
    }

    #[cfg(unix)]
    #[test]
    fn the_blocking_client_reads_events_and_cancels() {
        use std::io::{BufRead, Write};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.sock");
        let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            assert_eq!(line, "{\"cmd\":\"pair\"}\n");
            let mut out = stream;
            out.write_all(b"{\"event\":\"code\",\"code\":\"042 917\"}\n")
                .unwrap();
            line.clear();
            reader.read_line(&mut line).unwrap();
            assert_eq!(line, "{\"cmd\":\"answer\",\"accept\":true}\n");
            // The client cancels: the stream ends.
            line.clear();
            assert_eq!(reader.read_line(&mut line).unwrap(), 0);
        });
        let mut client = Client::connect(&path).unwrap();
        client.send(&Request::Pair).unwrap();
        assert_eq!(
            client.next_event().unwrap(),
            Some(Event::Code {
                code: "042 917".into()
            })
        );
        let mut handle = client.handle().unwrap();
        handle.answer(true).unwrap();
        handle.cancel();
        assert_eq!(client.next_event().unwrap(), None);
        server.join().unwrap();
    }
}
