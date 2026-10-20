//! Async read/write of length-framed `deck_proto` frames over any byte stream.

use deck_proto::{encode, Frame, FrameDecoder};
use std::io::{Error, ErrorKind};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Reads until one complete frame. `Ok(None)` on clean EOF between frames;
/// decode errors become `InvalidData`. Frames already buffered in `dec` are returned first.
pub async fn read_frame<R: AsyncRead + Unpin>(
    r: &mut R,
    dec: &mut FrameDecoder,
) -> std::io::Result<Option<Frame>> {
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        if let Some(f) = dec
            .next_frame()
            .map_err(|e| Error::new(ErrorKind::InvalidData, e))?
        {
            return Ok(Some(f));
        }
        let n = r.read(&mut buf).await?;
        if n == 0 {
            return Ok(None);
        }
        dec.push(&buf[..n]);
    }
}

/// Encodes and writes one frame; encode errors become `InvalidInput`.
pub async fn write_frame<W: AsyncWrite + Unpin>(w: &mut W, frame: &Frame) -> std::io::Result<()> {
    let bytes = encode(frame).map_err(|e| Error::new(ErrorKind::InvalidInput, e))?;
    w.write_all(&bytes).await?;
    w.flush().await
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;

    #[tokio::test]
    async fn round_trips_three_frames() {
        let (mut a, mut b) = tokio::io::duplex(1024);
        let frames = vec![
            Frame::Control(b"{}".to_vec()),
            Frame::Output {
                session: 1,
                data: b"out".to_vec(),
            },
            Frame::Input {
                session: 2,
                data: b"in".to_vec(),
            },
        ];
        for f in &frames {
            write_frame(&mut a, f).await.unwrap();
        }
        drop(a);
        let mut dec = FrameDecoder::new();
        for f in &frames {
            assert_eq!(&read_frame(&mut b, &mut dec).await.unwrap().unwrap(), f);
        }
        assert!(read_frame(&mut b, &mut dec).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn eof_gives_none() {
        let (a, mut b) = tokio::io::duplex(64);
        drop(a);
        let mut dec = FrameDecoder::new();
        assert!(read_frame(&mut b, &mut dec).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn bad_type_is_invalid_data() {
        let (mut a, mut b) = tokio::io::duplex(64);
        a.write_all(&[9, 0, 0, 0, 0]).await.unwrap();
        let mut dec = FrameDecoder::new();
        let e = read_frame(&mut b, &mut dec).await.unwrap_err();
        assert_eq!(e.kind(), std::io::ErrorKind::InvalidData);
    }
}
