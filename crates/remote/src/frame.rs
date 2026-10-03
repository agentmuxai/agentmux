// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The wire protocol between srv and a durable session (spec §7.2), carried
//! end to end over `ssh -T host -- agentmux-remote attach ...`: `attach` only
//! relays bytes between its stdio and the daemon's socket, so these frames go
//! straight from the daemon to srv and back.
//!
//! A frame is `[kind: u8][len: u32 big-endian][payload: len bytes]`. Output
//! carries the session's byte offset, so srv appends exactly what it has not
//! seen and a reattach asks for exactly what it missed.

/// Largest payload accepted; a longer length means a broken stream.
pub const MAX_PAYLOAD: usize = 1 << 20;

/// The protocol version `attach` and srv must share (the handshake line).
pub const PROTOCOL: u32 = 1;

/// One message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Frame {
    // srv -> session
    /// Keystrokes for the session's terminal.
    Input(Vec<u8>),
    /// The pane's size.
    Resize {
        cols: u16,
        rows: u16,
    },
    /// Are you there? Answered with `Pong`; silence means a stalled link (§7.5).
    Ping,
    /// Leave the session running and drop this connection.
    Detach,
    /// End the session: its shell and everything it started.
    End,

    // session -> srv
    /// Terminal output starting at byte `offset` of the session's stream.
    Output {
        offset: u64,
        data: Vec<u8>,
    },
    /// Bytes `from..to` fell out of the session's ring before they could be
    /// replayed: the pane marks a gap there.
    Lost {
        from: u64,
        to: u64,
    },
    /// The session's shell exited with `code`; the session is over.
    Exited {
        code: i32,
    },
    Pong,
    /// Sent first: the session id, whether it was just created, and the next
    /// offset its output will have.
    Hello {
        session: String,
        created: bool,
        end: u64,
    },
    /// Something went wrong, in words for the pane.
    Error(String),
}

mod kind {
    pub const INPUT: u8 = 1;
    pub const RESIZE: u8 = 2;
    pub const PING: u8 = 3;
    pub const DETACH: u8 = 4;
    pub const END: u8 = 5;
    pub const OUTPUT: u8 = 10;
    pub const LOST: u8 = 11;
    pub const EXITED: u8 = 12;
    pub const PONG: u8 = 13;
    pub const HELLO: u8 = 14;
    pub const ERROR: u8 = 15;
}

impl Frame {
    /// The frame's bytes on the wire.
    pub fn encode(&self) -> Vec<u8> {
        let (k, payload): (u8, Vec<u8>) = match self {
            Frame::Input(d) => (kind::INPUT, d.clone()),
            Frame::Resize { cols, rows } => {
                let mut p = cols.to_be_bytes().to_vec();
                p.extend_from_slice(&rows.to_be_bytes());
                (kind::RESIZE, p)
            }
            Frame::Ping => (kind::PING, Vec::new()),
            Frame::Detach => (kind::DETACH, Vec::new()),
            Frame::End => (kind::END, Vec::new()),
            Frame::Output { offset, data } => {
                let mut p = offset.to_be_bytes().to_vec();
                p.extend_from_slice(data);
                (kind::OUTPUT, p)
            }
            Frame::Lost { from, to } => {
                let mut p = from.to_be_bytes().to_vec();
                p.extend_from_slice(&to.to_be_bytes());
                (kind::LOST, p)
            }
            Frame::Exited { code } => (kind::EXITED, code.to_be_bytes().to_vec()),
            Frame::Pong => (kind::PONG, Vec::new()),
            Frame::Hello {
                session,
                created,
                end,
            } => {
                let mut p = end.to_be_bytes().to_vec();
                p.push(u8::from(*created));
                p.extend_from_slice(session.as_bytes());
                (kind::HELLO, p)
            }
            Frame::Error(msg) => (kind::ERROR, msg.as_bytes().to_vec()),
        };
        let mut out = Vec::with_capacity(5 + payload.len());
        out.push(k);
        out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        out.extend_from_slice(&payload);
        out
    }

    fn decode(k: u8, p: &[u8]) -> Result<Frame, String> {
        let u64_at = |i: usize| -> Result<u64, String> {
            p.get(i..i + 8)
                .map(|b| u64::from_be_bytes(b.try_into().unwrap()))
                .ok_or_else(|| format!("frame {k}: too short"))
        };
        Ok(match k {
            kind::INPUT => Frame::Input(p.to_vec()),
            kind::RESIZE => {
                if p.len() != 4 {
                    return Err("resize: wrong length".into());
                }
                Frame::Resize {
                    cols: u16::from_be_bytes([p[0], p[1]]),
                    rows: u16::from_be_bytes([p[2], p[3]]),
                }
            }
            kind::PING => Frame::Ping,
            kind::DETACH => Frame::Detach,
            kind::END => Frame::End,
            kind::OUTPUT => Frame::Output {
                offset: u64_at(0)?,
                data: p[8..].to_vec(),
            },
            kind::LOST => Frame::Lost {
                from: u64_at(0)?,
                to: u64_at(8)?,
            },
            kind::EXITED => {
                let b: [u8; 4] = p
                    .try_into()
                    .map_err(|_| "exited: wrong length".to_string())?;
                Frame::Exited {
                    code: i32::from_be_bytes(b),
                }
            }
            kind::PONG => Frame::Pong,
            kind::HELLO => {
                let end = u64_at(0)?;
                let created = *p.get(8).ok_or("hello: too short")? != 0;
                let session = String::from_utf8(p[9..].to_vec())
                    .map_err(|_| "hello: session id is not UTF-8")?;
                Frame::Hello {
                    session,
                    created,
                    end,
                }
            }
            kind::ERROR => Frame::Error(String::from_utf8_lossy(p).into_owned()),
            other => return Err(format!("unknown frame kind {other}")),
        })
    }
}

/// Reassembles frames from a byte stream that arrives in arbitrary pieces.
#[derive(Debug, Default)]
pub struct Decoder {
    buf: Vec<u8>,
}

impl Decoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add bytes; return every frame now complete. `Err` for a stream that is
    /// not frames (an unknown kind or an impossible length): the connection
    /// should be dropped.
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<Frame>, String> {
        self.buf.extend_from_slice(bytes);
        let mut frames = Vec::new();
        loop {
            if self.buf.len() < 5 {
                break;
            }
            let len =
                u32::from_be_bytes([self.buf[1], self.buf[2], self.buf[3], self.buf[4]]) as usize;
            if len > MAX_PAYLOAD {
                return Err(format!("frame of {len} bytes is over the limit"));
            }
            if self.buf.len() < 5 + len {
                break;
            }
            let frame = Frame::decode(self.buf[0], &self.buf[5..5 + len])?;
            self.buf.drain(..5 + len);
            frames.push(frame);
        }
        Ok(frames)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all() -> Vec<Frame> {
        vec![
            Frame::Input(b"ls -la\r".to_vec()),
            Frame::Resize {
                cols: 200,
                rows: 50,
            },
            Frame::Ping,
            Frame::Detach,
            Frame::End,
            Frame::Output {
                offset: 1 << 40,
                data: b"\x1b[31mred\x1b[0m".to_vec(),
            },
            Frame::Lost {
                from: 10,
                to: 8_000_000,
            },
            Frame::Exited { code: -1 },
            Frame::Pong,
            Frame::Hello {
                session: "s-1".into(),
                created: true,
                end: 42,
            },
            Frame::Error("no such session".into()),
        ]
    }

    #[test]
    fn every_frame_round_trips() {
        for f in all() {
            let mut d = Decoder::new();
            assert_eq!(d.push(&f.encode()).unwrap(), vec![f.clone()], "{f:?}");
        }
    }

    #[test]
    fn frames_split_anywhere_and_joined_reassemble() {
        let stream: Vec<u8> = all().iter().flat_map(Frame::encode).collect();
        for chunk in [1, 2, 3, 7, 64] {
            let mut d = Decoder::new();
            let mut got = Vec::new();
            for piece in stream.chunks(chunk) {
                got.extend(d.push(piece).unwrap());
            }
            assert_eq!(got, all(), "chunk {chunk}");
        }
    }

    #[test]
    fn a_stream_that_is_not_frames_is_an_error() {
        assert!(
            Decoder::new().push(&[99, 0, 0, 0, 0]).is_err(),
            "unknown kind"
        );
        assert!(
            Decoder::new().push(&[1, 0xff, 0xff, 0xff, 0xff]).is_err(),
            "impossible length"
        );
        assert!(
            Decoder::new().push(&[2, 0, 0, 0, 1, 7]).is_err(),
            "bad resize"
        );
    }
}
