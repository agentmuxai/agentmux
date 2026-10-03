// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A session's output history: the last `capacity` bytes of everything its
//! terminal wrote, addressed by offset in the whole stream (spec §7.2). A
//! reattach asks for "everything after offset N" and gets it, or, when N has
//! fallen out, everything still held plus how much was lost.
//!
//! Held in memory (8 MB by default per session): the spec's on-disk ring
//! would outlive the daemon, but the sessions themselves cannot (their
//! terminals are the daemon's), so memory loses nothing that disk would keep.

/// The default history per session.
#[cfg_attr(not(unix), allow(dead_code))]
pub const DEFAULT_CAPACITY: usize = 8 * 1024 * 1024;

#[derive(Debug)]
pub struct Ring {
    /// A deque, so dropping the oldest bytes is cheap even when full.
    buf: std::collections::VecDeque<u8>,
    capacity: usize,
    /// Offset of the next byte to be written: the stream's length so far.
    end: u64,
}

/// What a read from an offset gives back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Replay {
    /// Bytes `lost.0..lost.1` were asked for but are no longer held.
    pub lost: Option<(u64, u64)>,
    /// The offset of `data[0]`.
    pub offset: u64,
    pub data: Vec<u8>,
}

impl Ring {
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 0);
        Ring {
            buf: std::collections::VecDeque::new(),
            capacity,
            end: 0,
        }
    }

    /// The offset the next byte will have.
    pub fn end(&self) -> u64 {
        self.end
    }

    /// The oldest offset still held.
    pub fn start(&self) -> u64 {
        self.end - self.buf.len() as u64
    }

    pub fn append(&mut self, bytes: &[u8]) {
        self.end += bytes.len() as u64;
        if bytes.len() >= self.capacity {
            self.buf.clear();
            self.buf.extend(&bytes[bytes.len() - self.capacity..]);
            return;
        }
        let overflow = (self.buf.len() + bytes.len()).saturating_sub(self.capacity);
        self.buf.drain(..overflow);
        self.buf.extend(bytes);
    }

    /// Everything from `offset` on. An offset past the end is the end (nothing
    /// to replay); one before the start replays from the start and says what
    /// was lost.
    pub fn read_from(&self, offset: u64) -> Replay {
        let offset = offset.min(self.end);
        let start = self.start();
        if offset < start {
            return Replay {
                lost: Some((offset, start)),
                offset: start,
                data: self.buf.iter().copied().collect(),
            };
        }
        let skip = (offset - start) as usize;
        Replay {
            lost: None,
            offset,
            data: self.buf.range(skip..).copied().collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reattach_gets_exactly_what_it_missed() {
        let mut r = Ring::new(100);
        r.append(b"hello ");
        r.append(b"world");
        assert_eq!((r.start(), r.end()), (0, 11));
        assert_eq!(
            r.read_from(6),
            Replay {
                lost: None,
                offset: 6,
                data: b"world".to_vec()
            }
        );
        assert_eq!(r.read_from(0).data, b"hello world");
        assert_eq!(r.read_from(11).data, b"", "caught up: nothing");
        assert_eq!(
            r.read_from(500),
            Replay {
                lost: None,
                offset: 11,
                data: Vec::new()
            },
            "past the end is the end"
        );
    }

    #[test]
    fn what_fell_out_is_reported_not_silently_skipped() {
        let mut r = Ring::new(8);
        r.append(b"0123456789"); // longer than the ring
        assert_eq!((r.start(), r.end()), (2, 10));
        assert_eq!(
            r.read_from(0),
            Replay {
                lost: Some((0, 2)),
                offset: 2,
                data: b"23456789".to_vec()
            }
        );
        r.append(b"abc");
        assert_eq!((r.start(), r.end()), (5, 13));
        assert_eq!(
            r.read_from(3),
            Replay {
                lost: Some((3, 5)),
                offset: 5,
                data: b"56789abc".to_vec()
            }
        );
        assert_eq!(r.read_from(9).data, b"9abc");
    }

    #[test]
    fn many_small_writes_keep_the_last_capacity_bytes() {
        let mut r = Ring::new(16);
        let mut all = Vec::new();
        for i in 0..200u32 {
            let s = format!("{i},");
            r.append(s.as_bytes());
            all.extend_from_slice(s.as_bytes());
        }
        let replay = r.read_from(r.start());
        assert_eq!(replay.data, all[all.len() - 16..]);
        assert_eq!(r.end(), all.len() as u64);
    }
}
