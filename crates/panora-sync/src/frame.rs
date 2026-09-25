// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! Framing for the sync channel: a small JSON message plus any number of
//! binary blobs (sealed records), so ciphertext is not base64-inflated.
//!
//! ```text
//! u32 json_len | json | u32 blob_count | (u32 len | bytes) * blob_count
//! ```
//!
//! All integers are big-endian. The reader enforces a total size limit
//! before allocating anything.

use crate::error::{Error, Result};
use serde::de::DeserializeOwned;
use serde::Serialize;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Most blobs in one frame.
pub const MAX_BLOBS: usize = 64;

/// Write one frame.
pub async fn write<W, T>(writer: &mut W, message: &T, blobs: &[Vec<u8>]) -> Result<()>
where
    W: AsyncWrite + Unpin,
    T: Serialize,
{
    if blobs.len() > MAX_BLOBS {
        return Err(Error::Protocol("too many blobs in one frame"));
    }
    let json = serde_json::to_vec(message)?;
    let mut head = Vec::with_capacity(json.len() + 8);
    head.extend_from_slice(&len32(json.len())?.to_be_bytes());
    head.extend_from_slice(&json);
    head.extend_from_slice(&(blobs.len() as u32).to_be_bytes());
    writer.write_all(&head).await?;
    for blob in blobs {
        writer.write_all(&len32(blob.len())?.to_be_bytes()).await?;
        writer.write_all(blob).await?;
    }
    writer.flush().await?;
    Ok(())
}

/// Read one frame of at most `max_bytes` in total.
pub async fn read<R, T>(reader: &mut R, max_bytes: usize) -> Result<(T, Vec<Vec<u8>>)>
where
    R: AsyncRead + Unpin,
    T: DeserializeOwned,
{
    let mut budget = max_bytes;
    let json = read_chunk(reader, &mut budget).await?;
    let message = serde_json::from_slice(&json)?;
    let count = reader.read_u32().await? as usize;
    if count > MAX_BLOBS {
        return Err(Error::Protocol("too many blobs in one frame"));
    }
    let mut blobs = Vec::with_capacity(count);
    for _ in 0..count {
        blobs.push(read_chunk(reader, &mut budget).await?);
    }
    Ok((message, blobs))
}

async fn read_chunk<R: AsyncRead + Unpin>(reader: &mut R, budget: &mut usize) -> Result<Vec<u8>> {
    let len = reader.read_u32().await? as usize;
    if len > *budget {
        return Err(Error::Protocol("frame too large"));
    }
    *budget -= len;
    // Grow with what actually arrives instead of allocating the announced
    // length up front.
    let mut buf = Vec::with_capacity(len.min(64 * 1024));
    let got = reader.take(len as u64).read_to_end(&mut buf).await?;
    if got != len {
        return Err(Error::Io(std::io::ErrorKind::UnexpectedEof.into()));
    }
    Ok(buf)
}

fn len32(len: usize) -> Result<u32> {
    u32::try_from(len).map_err(|_| Error::Protocol("frame too large"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn round_trip_and_limits() {
        let (mut a, mut b) = tokio::io::duplex(1 << 16);
        write(&mut a, &"hello", &[b"one".to_vec(), vec![]])
            .await
            .unwrap();
        let (msg, blobs): (String, _) = read(&mut b, 1024).await.unwrap();
        assert_eq!(msg, "hello");
        assert_eq!(blobs, vec![b"one".to_vec(), vec![]]);

        write(&mut a, &"x", &[vec![0u8; 600], vec![0u8; 600]])
            .await
            .unwrap();
        assert!(matches!(
            read::<_, String>(&mut b, 1000).await,
            Err(Error::Protocol(_))
        ));
    }

    #[tokio::test]
    async fn too_many_blobs_is_refused() {
        let (mut a, _b) = tokio::io::duplex(1 << 16);
        let blobs = vec![Vec::new(); MAX_BLOBS + 1];
        assert!(write(&mut a, &"x", &blobs).await.is_err());
    }
}
