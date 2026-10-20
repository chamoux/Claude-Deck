//! Framing for the pipe between the window, the helper and the hook command
//! (spec §3.4): 1 byte type, 4 byte little-endian length, then the payload.
//! Terminal frames start their payload with a 4 byte session index.
//! The helper answers each hook message with the one-byte `HOOK_ACK` (spec §4.1).

pub mod messages;

/// Largest payload accepted. Anything bigger means a broken or hostile peer.
pub const MAX_PAYLOAD: usize = 16 * 1024 * 1024;

/// Byte the helper sends back to the hook command after applying its event (spec §4.1).
pub const HOOK_ACK: u8 = 0x06;

const TYPE_CONTROL: u8 = 1;
const TYPE_OUTPUT: u8 = 2;
const TYPE_INPUT: u8 = 3;
const HEADER: usize = 5;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Frame {
    /// A JSON control message.
    Control(Vec<u8>),
    /// Terminal output of one session, helper → window.
    Output { session: u32, data: Vec<u8> },
    /// Keyboard input for one session, window → helper.
    Input { session: u32, data: Vec<u8> },
}

/// After any error the stream is out of sync: drop the connection.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FrameError {
    #[error("unknown frame type {0}")]
    UnknownType(u8),
    #[error("frame payload of {0} bytes is over the limit")]
    TooLarge(usize),
    #[error("terminal frame is shorter than its 4 byte session index")]
    MissingSession,
}

pub fn encode(frame: &Frame) -> Result<Vec<u8>, FrameError> {
    let (kind, payload) = match frame {
        Frame::Control(json) => (TYPE_CONTROL, json.clone()),
        Frame::Output { session, data } => (TYPE_OUTPUT, with_session(*session, data)),
        Frame::Input { session, data } => (TYPE_INPUT, with_session(*session, data)),
    };
    if payload.len() > MAX_PAYLOAD {
        return Err(FrameError::TooLarge(payload.len()));
    }
    let mut out = Vec::with_capacity(HEADER + payload.len());
    out.push(kind);
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(&payload);
    Ok(out)
}

/// Collects bytes from a stream and hands out complete frames.
#[derive(Debug, Default)]
pub struct FrameDecoder {
    buf: Vec<u8>,
}

impl FrameDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    /// The next complete frame, or `None` until more bytes arrive.
    pub fn next_frame(&mut self) -> Result<Option<Frame>, FrameError> {
        if self.buf.len() < HEADER {
            return Ok(None);
        }
        let kind = self.buf[0];
        let len = u32::from_le_bytes([self.buf[1], self.buf[2], self.buf[3], self.buf[4]]) as usize;
        if len > MAX_PAYLOAD {
            return Err(FrameError::TooLarge(len));
        }
        if !matches!(kind, TYPE_CONTROL | TYPE_OUTPUT | TYPE_INPUT) {
            return Err(FrameError::UnknownType(kind));
        }
        if self.buf.len() < HEADER + len {
            return Ok(None);
        }
        let payload = self.buf[HEADER..HEADER + len].to_vec();
        self.buf.drain(..HEADER + len);
        let frame = match kind {
            TYPE_CONTROL => Frame::Control(payload),
            TYPE_OUTPUT => {
                let (session, data) = split_session(payload)?;
                Frame::Output { session, data }
            }
            _ => {
                let (session, data) = split_session(payload)?;
                Frame::Input { session, data }
            }
        };
        Ok(Some(frame))
    }
}

fn with_session(session: u32, data: &[u8]) -> Vec<u8> {
    let mut p = Vec::with_capacity(4 + data.len());
    p.extend_from_slice(&session.to_le_bytes());
    p.extend_from_slice(data);
    p
}

fn split_session(mut payload: Vec<u8>) -> Result<(u32, Vec<u8>), FrameError> {
    if payload.len() < 4 {
        return Err(FrameError::MissingSession);
    }
    let session = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
    payload.drain(..4);
    Ok((session, payload))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_frames() -> Vec<Frame> {
        vec![
            Frame::Control(br#"{"type":"get_state"}"#.to_vec()),
            Frame::Output {
                session: 7,
                data: b"\x1b[32mhello\x1b[0m".to_vec(),
            },
            Frame::Input {
                session: 0,
                data: b"\r".to_vec(),
            },
            Frame::Control(Vec::new()),
        ]
    }

    #[test]
    fn frames_round_trip() {
        for frame in all_frames() {
            let mut d = FrameDecoder::new();
            d.push(&encode(&frame).unwrap());
            assert_eq!(d.next_frame().unwrap(), Some(frame));
            assert_eq!(d.next_frame().unwrap(), None);
        }
    }

    #[test]
    fn header_is_type_then_little_endian_length() {
        assert_eq!(
            encode(&Frame::Control(b"{}".to_vec())).unwrap(),
            vec![1, 2, 0, 0, 0, b'{', b'}']
        );
        assert_eq!(
            encode(&Frame::Output {
                session: 1,
                data: b"A".to_vec()
            })
            .unwrap(),
            vec![2, 5, 0, 0, 0, 1, 0, 0, 0, b'A']
        );
    }

    #[test]
    fn decodes_when_bytes_arrive_one_at_a_time() {
        let bytes: Vec<u8> = all_frames()
            .iter()
            .flat_map(|f| encode(f).unwrap())
            .collect();
        let mut d = FrameDecoder::new();
        let mut got = Vec::new();
        for b in bytes {
            d.push(&[b]);
            while let Some(f) = d.next_frame().unwrap() {
                got.push(f);
            }
        }
        assert_eq!(got, all_frames());
    }

    #[test]
    fn rejects_bad_frames() {
        let mut d = FrameDecoder::new();
        d.push(&[9, 0, 0, 0, 0]);
        assert_eq!(d.next_frame(), Err(FrameError::UnknownType(9)));

        let mut d = FrameDecoder::new();
        d.push(&[1]);
        d.push(&(MAX_PAYLOAD as u32 + 1).to_le_bytes());
        assert_eq!(d.next_frame(), Err(FrameError::TooLarge(MAX_PAYLOAD + 1)));

        let mut d = FrameDecoder::new();
        d.push(&[2, 2, 0, 0, 0, 1, 0]);
        assert_eq!(d.next_frame(), Err(FrameError::MissingSession));
    }

    #[test]
    fn encode_refuses_an_oversized_payload() {
        let big = Frame::Output {
            session: 1,
            data: vec![0; MAX_PAYLOAD],
        }; // + 4 byte prefix = over
        assert_eq!(encode(&big), Err(FrameError::TooLarge(MAX_PAYLOAD + 4)));
        assert!(encode(&Frame::Control(vec![0; MAX_PAYLOAD])).is_ok());
    }
}
