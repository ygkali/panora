// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! Protocol v3 wire framing (STO-08).
//!
//! v2 puts one JSON object per line on the wire and base64-encodes every
//! [`crate::model::MimePayload`] inside it, which both quadruples-over-three
//! the bytes on the wire for anything large (images) and forces a hard cap
//! on how big a reply can be (`ipc::MAX_RESPONSE_BYTES`) so a client never
//! has to allocate without bound. v3 keeps the JSON for the small,
//! structured part of a message (the request/response shape itself) but
//! carries payload bytes out of band: inline as raw bytes for anything
//! small, or as a `memfd_create` file descriptor passed over the socket via
//! `SCM_RIGHTS` for anything large. That removes both problems at once —
//! no base64 inflation, and no size ceiling other than free memory.
//!
//! ## Wire format
//!
//! A v3 connection opens with a 4-byte magic (see [`MAGIC`]) that a v2
//! JSON-lines request can never start with (every v2 request is a JSON
//! object, so its first byte is always `{`). The daemon tells v2 and v3
//! clients apart with a single non-consuming (`MSG_PEEK`) read of that first
//! byte; see `peek_is_v3`.
//!
//! After the magic, frames follow, each:
//!
//! ```text
//! u32 header_len          (little-endian)
//! header_len bytes        JSON: FrameHeader<T> { body: T, payloads: [..] }
//! u32 inline_len          (little-endian)
//! inline_len bytes        concatenation of every PayloadRef::Inline chunk,
//!                         in the order `payloads` lists them
//! ```
//!
//! Payloads over [`INLINE_LIMIT`] are omitted from the inline blob; instead
//! their bytes live in an anonymous `memfd`, and its file descriptor rides
//! as `SCM_RIGHTS` ancillary data on the same `sendmsg` call that carries
//! the frame's first byte. The fds for one frame arrive, in order, on
//! whichever `recvmsg` call first reads that byte — the standard guarantee
//! Linux gives for `SCM_RIGHTS` on `SOCK_STREAM`, which holds as long as the
//! receiver always reads through `recvmsg` and never falls back to a plain
//! `read`. This module never does.

use super::{Event, Request, Response, ResponseData};
use crate::error::{Error, Result};
use rustix::fd::{AsFd, BorrowedFd, OwnedFd};
use rustix::net::{
    recv, RecvAncillaryBuffer, RecvAncillaryMessage, RecvFlags, SendAncillaryBuffer,
    SendAncillaryMessage, SendFlags,
};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::io::{IoSlice, IoSliceMut, Read, Seek, SeekFrom, Write};

/// Connection preamble that marks a v3 client. A v2 request is always a
/// JSON object (`{...}`), so its first byte is `{` (0x7B) and can never
/// collide with this.
pub const MAGIC: &[u8; 4] = b"PNR3";

/// Payloads at or under this size travel inline in the frame instead of
/// through a passed file descriptor: one syscall, one allocation, no memfd,
/// for the common case of short text clips.
pub const INLINE_LIMIT: usize = 8 * 1024;

/// Upper bound on one frame's header + inline bytes. Exists for the same
/// reason `ipc::MAX_FRAME_BYTES` exists on v2: an unbounded read would let a
/// peer make this process allocate without limit. Large payloads do not
/// count against this — they travel as fds, not inline bytes.
pub const MAX_FRAME_BYTES: u32 = 16 * 1024 * 1024;

/// A frame carries at most this many out-of-line (fd-backed) payloads. A
/// clipboard entry realistically offers a handful of MIME formats; this
/// bounds the ancillary-message buffer to a fixed, small size.
pub const MAX_FDS_PER_FRAME: usize = 16;

/// How a payload's bytes were carried for one frame.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
enum PayloadRef {
    /// `len` bytes at the next unread offset of the frame's inline blob.
    Inline { len: u32 },
    /// The next unread passed file descriptor, `len` bytes long.
    Fd { len: u64 },
}

#[derive(Serialize)]
struct FrameHeaderOut<'a, T: Serialize> {
    body: &'a T,
    payloads: &'a [PayloadRef],
}

#[derive(Deserialize)]
struct FrameHeaderIn<T> {
    body: T,
    payloads: Vec<PayloadRef>,
}

/// One decoded frame: the header value with its payload bytes reassembled
/// in declared order, ready for `restore_*_payloads`.
struct RecvFrame<T> {
    body: T,
    payload_bytes: VecDeque<Vec<u8>>,
}

// --- payload extraction/restoration: the only two message shapes that ever
// carry a MimePayload are Request::Store and ResponseData::Payloads. ---

fn take_request_payloads(request: &mut Request) -> Vec<Vec<u8>> {
    match request {
        Request::Store { payloads, .. } => payloads
            .iter_mut()
            .map(|p| std::mem::take(&mut p.data))
            .collect(),
        _ => Vec::new(),
    }
}

fn restore_request_payloads(request: &mut Request, mut bytes: VecDeque<Vec<u8>>) {
    if let Request::Store { payloads, .. } = request {
        for p in payloads.iter_mut() {
            if let Some(b) = bytes.pop_front() {
                p.data = b;
            }
        }
    }
}

fn take_response_payloads(response: &mut Response) -> Vec<Vec<u8>> {
    if let Response::Success(ResponseData::Payloads(payloads)) = response {
        payloads
            .iter_mut()
            .map(|p| std::mem::take(&mut p.data))
            .collect()
    } else {
        Vec::new()
    }
}

fn restore_response_payloads(response: &mut Response, mut bytes: VecDeque<Vec<u8>>) {
    if let Response::Success(ResponseData::Payloads(payloads)) = response {
        for p in payloads.iter_mut() {
            if let Some(b) = bytes.pop_front() {
                p.data = b;
            }
        }
    }
}

/// Build one anonymous, sealed-to-this-write memfd holding `bytes`, rewound
/// to its start so the receiver can read it straight through.
fn memfd_payload(bytes: &[u8]) -> Result<OwnedFd> {
    use rustix::fs::{memfd_create, MemfdFlags};
    let fd = memfd_create(c"panora-ipc-payload", MemfdFlags::CLOEXEC)
        .map_err(|e| Error::Ipc(format!("memfd_create: {e}")))?;
    let mut file = std::fs::File::from(fd);
    file.write_all(bytes)
        .map_err(|e| Error::Ipc(format!("memfd write: {e}")))?;
    file.seek(SeekFrom::Start(0))
        .map_err(|e| Error::Ipc(format!("memfd seek: {e}")))?;
    Ok(OwnedFd::from(file))
}

/// Serialize `body` plus its already-extracted payload bytes into one
/// frame's wire bytes, creating a memfd for every payload over
/// [`INLINE_LIMIT`]. Returns the bytes to write and the fds to pass
/// alongside them, in `PayloadRef::Fd` order.
fn build_frame<T: Serialize>(
    body: &T,
    payload_bytes: &[Vec<u8>],
) -> Result<(Vec<u8>, Vec<OwnedFd>)> {
    if payload_bytes.len() > MAX_FDS_PER_FRAME {
        return Err(Error::Ipc("too many payloads for one v3 frame".into()));
    }
    let mut payloads = Vec::with_capacity(payload_bytes.len());
    let mut inline = Vec::new();
    let mut fds = Vec::new();
    for bytes in payload_bytes {
        if bytes.len() <= INLINE_LIMIT {
            payloads.push(PayloadRef::Inline {
                len: bytes.len() as u32,
            });
            inline.extend_from_slice(bytes);
        } else {
            fds.push(memfd_payload(bytes)?);
            payloads.push(PayloadRef::Fd {
                len: bytes.len() as u64,
            });
        }
    }
    let header_json = serde_json::to_vec(&FrameHeaderOut {
        body,
        payloads: &payloads,
    })
    .map_err(|e| Error::Ipc(format!("v3 header encode: {e}")))?;
    if header_json.len() as u64 + inline.len() as u64 > MAX_FRAME_BYTES as u64 {
        return Err(Error::Ipc("v3 frame exceeds the size limit".into()));
    }
    let mut out = Vec::with_capacity(4 + header_json.len() + 4 + inline.len());
    out.extend_from_slice(&(header_json.len() as u32).to_le_bytes());
    out.extend_from_slice(&header_json);
    out.extend_from_slice(&(inline.len() as u32).to_le_bytes());
    out.extend_from_slice(&inline);
    Ok((out, fds))
}

/// Reassemble a decoded header's payload bytes (inline blob + received fds)
/// back into `Vec<u8>`s in declared order.
fn assemble_payloads(
    payloads: &[PayloadRef],
    inline: &[u8],
    fds: Vec<OwnedFd>,
) -> Result<VecDeque<Vec<u8>>> {
    let mut cursor = 0usize;
    let mut fd_iter = fds.into_iter();
    let mut out = VecDeque::with_capacity(payloads.len());
    for p in payloads {
        match *p {
            PayloadRef::Inline { len } => {
                let len = len as usize;
                let end = cursor
                    .checked_add(len)
                    .filter(|&e| e <= inline.len())
                    .ok_or_else(|| Error::Ipc("v3 frame: inline payload out of range".into()))?;
                out.push_back(inline[cursor..end].to_vec());
                cursor = end;
            }
            PayloadRef::Fd { len } => {
                let fd = fd_iter
                    .next()
                    .ok_or_else(|| Error::Ipc("v3 frame: missing passed file descriptor".into()))?;
                let mut file = std::fs::File::from(fd);
                let mut bytes = Vec::with_capacity(len.min(64 * 1024 * 1024) as usize);
                (&mut file)
                    .take(len)
                    .read_to_end(&mut bytes)
                    .map_err(|e| Error::Ipc(format!("v3 frame: reading passed fd: {e}")))?;
                out.push_back(bytes);
            }
        }
    }
    Ok(out)
}

// --- single-syscall primitives, shared by the blocking client and the
// async server (which wraps these in `try_io`). ---

/// One non-blocking `sendmsg` attempt: at most one syscall. `fds` is
/// attached as `SCM_RIGHTS` (ignored if empty); pass fds only on the first
/// call for a given frame, since they only need to ride with its first
/// byte.
pub(crate) fn send_once<S: AsFd>(
    sock: &S,
    bytes: &[u8],
    fds: &[BorrowedFd<'_>],
) -> std::io::Result<usize> {
    let iov = [IoSlice::new(bytes)];
    let mut space = [std::mem::MaybeUninit::<u8>::uninit(); 256];
    let mut ancillary = SendAncillaryBuffer::new(&mut space);
    if !fds.is_empty() {
        ancillary.push(SendAncillaryMessage::ScmRights(fds));
    }
    rustix::net::sendmsg(sock, &iov, &mut ancillary, SendFlags::NOSIGNAL)
        .map_err(std::io::Error::from)
}

/// One non-blocking `recvmsg` attempt: at most one syscall. Any `SCM_RIGHTS`
/// fds it receives are appended to `fds_out`, regardless of how many bytes
/// (even zero, on a short read) came back with them.
pub(crate) fn recv_once<S: AsFd>(
    sock: &S,
    buf: &mut [u8],
    fds_out: &mut Vec<OwnedFd>,
) -> std::io::Result<usize> {
    let mut iov = [IoSliceMut::new(buf)];
    let mut space = [std::mem::MaybeUninit::<u8>::uninit(); 256];
    let mut ancillary = RecvAncillaryBuffer::new(&mut space);
    let result = rustix::net::recvmsg(sock, &mut iov, &mut ancillary, RecvFlags::CMSG_CLOEXEC)
        .map_err(std::io::Error::from)?;
    for msg in ancillary.drain() {
        if let RecvAncillaryMessage::ScmRights(rights) = msg {
            fds_out.extend(rights);
        }
    }
    Ok(result.bytes)
}

/// One non-consuming (`MSG_PEEK`) read of the connection's very first byte:
/// `Some(true)` if it opens a v3 connection (matches `MAGIC[0]`), `Some(false)`
/// for a v2 JSON-lines request, `None` if the peer closed before sending
/// anything. Never consumes the byte — the v2 path re-reads it normally.
pub fn peek_first_byte<S: AsFd>(sock: &S) -> std::io::Result<Option<u8>> {
    let mut buf = [0u8; 1];
    let (n, _) = recv(sock, &mut buf, RecvFlags::PEEK).map_err(std::io::Error::from)?;
    Ok((n > 0).then_some(buf[0]))
}

// --- blocking layer: used directly by the client (CLI/GUI), which talks to
// the daemon over a short-lived plain `std::os::unix::net::UnixStream`. ---

fn send_frame_blocking<S: AsFd>(sock: &S, bytes: &[u8], fds: &[OwnedFd]) -> std::io::Result<()> {
    let borrowed: Vec<BorrowedFd<'_>> = fds.iter().map(AsFd::as_fd).collect();
    let mut sent = 0usize;
    while sent < bytes.len() {
        let attach: &[BorrowedFd<'_>] = if sent == 0 { &borrowed } else { &[] };
        match send_once(sock, &bytes[sent..], attach) {
            Ok(0) => return Err(std::io::ErrorKind::WriteZero.into()),
            Ok(n) => sent += n,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

fn recv_exact_blocking<S: AsFd>(
    sock: &S,
    mut buf: &mut [u8],
    fds: &mut Vec<OwnedFd>,
) -> std::io::Result<()> {
    while !buf.is_empty() {
        match recv_once(sock, buf, fds) {
            Ok(0) => return Err(std::io::ErrorKind::UnexpectedEof.into()),
            Ok(n) => {
                let tmp = buf;
                buf = &mut tmp[n..];
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

fn recv_frame_blocking<S: AsFd, T: for<'de> Deserialize<'de>>(sock: &S) -> Result<RecvFrame<T>> {
    let io_err = |e: std::io::Error| Error::Ipc(format!("v3 read: {e}"));
    let mut fds = Vec::new();
    let mut len_buf = [0u8; 4];
    recv_exact_blocking(sock, &mut len_buf, &mut fds).map_err(io_err)?;
    let header_len = u32::from_le_bytes(len_buf);
    if header_len > MAX_FRAME_BYTES {
        return Err(Error::Ipc("v3 frame header exceeds the size limit".into()));
    }
    let mut header_buf = vec![0u8; header_len as usize];
    recv_exact_blocking(sock, &mut header_buf, &mut fds).map_err(io_err)?;
    let header: FrameHeaderIn<T> = serde_json::from_slice(&header_buf)
        .map_err(|e| Error::Ipc(format!("v3 header decode: {e}")))?;

    recv_exact_blocking(sock, &mut len_buf, &mut fds).map_err(io_err)?;
    let inline_len = u32::from_le_bytes(len_buf);
    if header_len as u64 + inline_len as u64 > MAX_FRAME_BYTES as u64 {
        return Err(Error::Ipc("v3 frame exceeds the size limit".into()));
    }
    let mut inline_buf = vec![0u8; inline_len as usize];
    recv_exact_blocking(sock, &mut inline_buf, &mut fds).map_err(io_err)?;

    let payload_bytes = assemble_payloads(&header.payloads, &inline_buf, fds)?;
    Ok(RecvFrame {
        body: header.body,
        payload_bytes,
    })
}

/// Write the v3 connection preamble. Must be the first thing a v3 client
/// sends.
pub fn write_magic_blocking<S: AsFd>(sock: &S) -> Result<()> {
    send_frame_blocking(sock, MAGIC, &[]).map_err(|e| Error::Ipc(format!("v3 magic: {e}")))
}

/// Send one `Request` as a v3 frame.
pub fn write_request_blocking<S: AsFd>(sock: &S, mut request: Request) -> Result<()> {
    let payload_bytes = take_request_payloads(&mut request);
    let (bytes, fds) = build_frame(&request, &payload_bytes)?;
    send_frame_blocking(sock, &bytes, &fds).map_err(|e| Error::Ipc(format!("v3 write: {e}")))
}

/// Read one `Response` v3 frame (a reply to a request just sent).
pub fn read_response_blocking<S: AsFd>(sock: &S) -> Result<Response> {
    let mut frame: RecvFrame<Response> = recv_frame_blocking(sock)?;
    restore_response_payloads(&mut frame.body, frame.payload_bytes);
    Ok(frame.body)
}

/// Read one `Event` v3 frame (used by `Subscribe` connections).
pub fn read_event_blocking<S: AsFd>(sock: &S) -> Result<Event> {
    let frame: RecvFrame<Event> = recv_frame_blocking(sock)?;
    Ok(frame.body)
}

/// Server-side counterparts of the above, exposed so `panod` can build its
/// own `try_io`-based async loop out of the same single-syscall primitives
/// (`send_once`/`recv_once`/`peek_first_byte`) plus these framing helpers.
pub mod server {
    use super::*;

    /// Everything needed to write one wire frame: bytes plus the fds that
    /// must ride along with their first byte.
    pub struct WireFrame {
        /// Bytes to write to the socket, in order.
        pub bytes: Vec<u8>,
        /// File descriptors to attach as `SCM_RIGHTS` to the write that
        /// sends `bytes`' first byte.
        pub fds: Vec<OwnedFd>,
    }

    /// Drives one `WireFrame` out to the socket one `send_once` attempt at a
    /// time, so an async caller can `await` writability between attempts
    /// instead of busy-looping on `WouldBlock`. Mirrors `FrameReader` on the
    /// write side.
    pub struct FrameWriter {
        bytes: Vec<u8>,
        fds: Vec<OwnedFd>,
        sent: usize,
    }

    impl FrameWriter {
        /// Start writing `frame`.
        pub fn new(frame: WireFrame) -> Self {
            Self {
                bytes: frame.bytes,
                fds: frame.fds,
                sent: 0,
            }
        }

        /// Whether every byte has been written.
        pub fn is_done(&self) -> bool {
            self.sent >= self.bytes.len()
        }

        /// One `sendmsg` attempt. `fds` are attached only on the very first
        /// call, since they only need to ride with the frame's first byte.
        pub fn poll_once<S: AsFd>(&mut self, sock: &S) -> std::io::Result<()> {
            let borrowed: Vec<BorrowedFd<'_>> = if self.sent == 0 {
                self.fds.iter().map(AsFd::as_fd).collect()
            } else {
                Vec::new()
            };
            let n = send_once(sock, &self.bytes[self.sent..], &borrowed)?;
            self.sent += n;
            Ok(())
        }
    }

    /// Build the wire frame for a `Response`.
    pub fn encode_response(mut response: Response) -> Result<WireFrame> {
        let payload_bytes = take_response_payloads(&mut response);
        let (bytes, fds) = build_frame(&response, &payload_bytes)?;
        Ok(WireFrame { bytes, fds })
    }

    /// Build the wire frame for an `Event`.
    pub fn encode_event(event: &Event) -> Result<WireFrame> {
        let (bytes, fds) = build_frame(event, &[])?;
        Ok(WireFrame { bytes, fds })
    }

    /// Build the wire frame for a `Hello` reply (kept separate so callers
    /// don't need to construct a full `Response` for it).
    pub fn encode_hello(protocol: u32) -> Result<WireFrame> {
        encode_response(Response::Success(ResponseData::Hello { protocol }))
    }

    /// Incrementally assembles one frame from bytes/fds handed to it by an
    /// async reader loop, one `recv_once` result at a time. Kept as a small
    /// state machine (rather than the blocking recursive reads above) so
    /// the caller can `await` readiness between calls instead of busy-
    /// looping on `WouldBlock`.
    pub struct FrameReader {
        stage: Stage,
        fds: Vec<OwnedFd>,
    }

    enum Stage {
        Len(Vec<u8>),
        Header {
            len: u32,
            buf: Vec<u8>,
        },
        InlineLen {
            header: Vec<u8>,
            buf: Vec<u8>,
        },
        Inline {
            header: Vec<u8>,
            len: u32,
            buf: Vec<u8>,
        },
        Done,
    }

    impl Default for FrameReader {
        fn default() -> Self {
            Self {
                stage: Stage::Len(Vec::with_capacity(4)),
                fds: Vec::new(),
            }
        }
    }

    impl FrameReader {
        /// Feed up to `want` bytes read via `recv_once` into the state
        /// machine, returning `Ok(Some(bytes_read))` once one `recv_once`
        /// call has been made (0 means peer closed), or the fully decoded
        /// frame once every stage has its bytes.
        pub fn poll_once<S: AsFd>(&mut self, sock: &S) -> std::io::Result<FramePoll> {
            if matches!(self.stage, Stage::Done) {
                return Ok(FramePoll::Pending);
            }
            let target = match &self.stage {
                Stage::Len(_) => 4,
                Stage::Header { len, .. } => *len as usize,
                Stage::InlineLen { .. } => 4,
                Stage::Inline { len, .. } => *len as usize,
                Stage::Done => unreachable!(),
            };
            let Self { stage, fds } = self;
            let buf = match stage {
                Stage::Len(buf)
                | Stage::Header { buf, .. }
                | Stage::InlineLen { buf, .. }
                | Stage::Inline { buf, .. } => buf,
                Stage::Done => unreachable!("checked above"),
            };
            if buf.len() < target {
                let start = buf.len();
                buf.resize(target, 0);
                let n = match recv_once(sock, &mut buf[start..], fds) {
                    Ok(n) => n,
                    Err(e) => {
                        buf.truncate(start);
                        return Err(e);
                    }
                };
                buf.truncate(start + n);
                if n == 0 {
                    return Ok(FramePoll::Closed);
                }
            }
            if buf.len() < target {
                return Ok(FramePoll::Pending);
            }
            // This stage's bytes are complete; advance.
            self.stage = match std::mem::replace(&mut self.stage, Stage::Done) {
                Stage::Len(buf) => {
                    let len = u32::from_le_bytes(buf.try_into().expect("4 bytes"));
                    if len > MAX_FRAME_BYTES {
                        return Err(std::io::Error::other(
                            "v3 frame header exceeds the size limit",
                        ));
                    }
                    Stage::Header {
                        len,
                        buf: Vec::with_capacity(len as usize),
                    }
                }
                Stage::Header { buf, .. } => Stage::InlineLen {
                    header: buf,
                    buf: Vec::with_capacity(4),
                },
                Stage::InlineLen { header, buf } => {
                    let len = u32::from_le_bytes(buf.try_into().expect("4 bytes"));
                    if header.len() as u64 + len as u64 > MAX_FRAME_BYTES as u64 {
                        return Err(std::io::Error::other("v3 frame exceeds the size limit"));
                    }
                    Stage::Inline {
                        header,
                        len,
                        buf: Vec::with_capacity(len as usize),
                    }
                }
                Stage::Inline { header, buf, .. } => {
                    return Ok(FramePoll::Ready {
                        header,
                        inline: buf,
                        fds: std::mem::take(&mut self.fds),
                    });
                }
                Stage::Done => unreachable!(),
            };
            Ok(FramePoll::Pending)
        }
    }

    /// Result of one `FrameReader::poll_once` call.
    pub enum FramePoll {
        /// Need more bytes; caller should await readability and retry.
        Pending,
        /// Peer closed the connection mid-frame.
        Closed,
        /// The frame is fully assembled.
        Ready {
            /// The frame's raw header JSON, not yet decoded.
            header: Vec<u8>,
            /// The frame's inline payload blob, not yet split up.
            inline: Vec<u8>,
            /// File descriptors received alongside this frame, in order.
            fds: Vec<OwnedFd>,
        },
    }

    /// Decode a `Ready` frame's header JSON and reassemble its payload
    /// bytes.
    pub fn decode<T>(header: &[u8], inline: &[u8], fds: Vec<OwnedFd>) -> Result<T>
    where
        T: for<'de> Deserialize<'de> + PayloadCarrier,
    {
        let decoded: FrameHeaderIn<T> = serde_json::from_slice(header)
            .map_err(|e| Error::Ipc(format!("v3 header decode: {e}")))?;
        let mut body = decoded.body;
        let bytes = assemble_payloads(&decoded.payloads, inline, fds)?;
        body.restore(bytes);
        Ok(body)
    }

    /// Implemented by the message types `decode` can restore payload bytes
    /// into after JSON decoding.
    pub trait PayloadCarrier {
        /// Splice extracted payload bytes back into their original fields.
        fn restore(&mut self, bytes: VecDeque<Vec<u8>>);
    }

    impl PayloadCarrier for Request {
        fn restore(&mut self, bytes: VecDeque<Vec<u8>>) {
            restore_request_payloads(self, bytes)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::QueryRequest;
    use crate::model::MimePayload;
    use std::os::unix::net::UnixStream;

    fn pair() -> (UnixStream, UnixStream) {
        UnixStream::pair().unwrap()
    }

    #[test]
    fn magic_is_never_a_valid_v2_first_byte() {
        // v2 requests always serialize to a JSON object.
        let req = Request::List(QueryRequest::default());
        let encoded = super::super::encode(&req).unwrap();
        assert_ne!(encoded[0], MAGIC[0]);
        assert_eq!(encoded[0], b'{');
    }

    #[test]
    fn small_payload_round_trips_inline() {
        let (a, b) = pair();
        let request = Request::Store {
            payloads: vec![MimePayload::new("text/plain", b"hello".to_vec())],
            source_app: None,
            copy: false,
        };
        write_request_blocking(&a, request).unwrap();
        let frame: RecvFrame<Request> = recv_frame_blocking(&b).unwrap();
        let mut body = frame.body;
        restore_request_payloads(&mut body, frame.payload_bytes);
        let Request::Store { payloads, .. } = body else {
            panic!("expected Store");
        };
        assert_eq!(payloads[0].data, b"hello");
    }

    #[test]
    fn large_payload_travels_as_a_passed_fd() {
        let (a, b) = pair();
        let big = vec![0x42u8; INLINE_LIMIT + 1];
        let request = Request::Store {
            payloads: vec![MimePayload::new("image/png", big.clone())],
            source_app: None,
            copy: false,
        };
        write_request_blocking(&a, request).unwrap();
        let frame: RecvFrame<Request> = recv_frame_blocking(&b).unwrap();
        let mut body = frame.body;
        restore_request_payloads(&mut body, frame.payload_bytes);
        let Request::Store { payloads, .. } = body else {
            panic!("expected Store");
        };
        assert_eq!(payloads[0].data, big);
    }

    #[test]
    fn mixed_inline_and_fd_payloads_keep_their_order() {
        let (a, b) = pair();
        let small = b"small".to_vec();
        let big = vec![0x7eu8; INLINE_LIMIT * 2];
        let request = Request::Store {
            payloads: vec![
                MimePayload::new("text/plain", small.clone()),
                MimePayload::new("image/png", big.clone()),
                MimePayload::new("text/html", small.clone()),
            ],
            source_app: None,
            copy: false,
        };
        write_request_blocking(&a, request).unwrap();
        let frame: RecvFrame<Request> = recv_frame_blocking(&b).unwrap();
        let mut body = frame.body;
        restore_request_payloads(&mut body, frame.payload_bytes);
        let Request::Store { payloads, .. } = body else {
            panic!("expected Store");
        };
        assert_eq!(payloads[0].data, small);
        assert_eq!(payloads[1].data, big);
        assert_eq!(payloads[2].data, small);
    }

    #[test]
    fn response_payloads_round_trip() {
        let (a, b) = pair();
        let response = Response::Success(ResponseData::Payloads(vec![MimePayload::new(
            "text/plain",
            b"reply".to_vec(),
        )]));
        let payload_bytes = {
            let mut r = response.clone();
            take_response_payloads(&mut r)
        };
        let (bytes, fds) = build_frame(&response, &payload_bytes).unwrap();
        send_frame_blocking(&a, &bytes, &fds).unwrap();
        let mut got = read_response_blocking(&b).unwrap();
        let Response::Success(ResponseData::Payloads(payloads)) = &mut got else {
            panic!("expected Payloads");
        };
        assert_eq!(payloads[0].data, b"reply");
    }

    #[test]
    fn event_round_trips() {
        let (a, b) = pair();
        let event = Event::Changed { revision: 7 };
        let (bytes, fds) = build_frame(&event, &[]).unwrap();
        send_frame_blocking(&a, &bytes, &fds).unwrap();
        let got = read_event_blocking(&b).unwrap();
        assert!(matches!(got, Event::Changed { revision: 7 }));
    }

    #[test]
    fn peek_leaves_the_byte_for_a_normal_read() {
        let (mut a, b) = pair();
        a.write_all(b"{\"x\":1}\n").unwrap();
        let first = peek_first_byte(&b).unwrap();
        assert_eq!(first, Some(b'{'));
        // The byte is still there for a real read.
        let mut buf = [0u8; 1];
        b.set_nonblocking(false).unwrap();
        use std::io::Read as _;
        (&b).read_exact(&mut buf).unwrap();
        assert_eq!(buf[0], b'{');
    }

    #[test]
    fn oversized_header_is_rejected() {
        let (mut a, b) = pair();
        let bytes = (MAX_FRAME_BYTES + 1).to_le_bytes();
        a.write_all(&bytes).unwrap();
        drop(a);
        let result: Result<RecvFrame<Request>> = recv_frame_blocking(&b);
        assert!(result.is_err());
    }
}
