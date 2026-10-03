// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The file protocol between srv and `agentmux-remote serve --stdio`
//! (SPEC_REMOTE_TERMINALS_AND_DURABLE_SESSIONS_2026_10_02.md §6.1, §6.3),
//! carried over one `ssh -T host -- agentmux-remote serve --stdio` per
//! connection. Shared by both ends, so they can never disagree on it.
//!
//! A message is `[len: u32 big-endian][body: len bytes]`. A request body is
//! `[id: u32][op: u8][fields]`, a reply body `[id: u32][status: u8][fields]`;
//! the reply carries its request's id. Strings and byte runs are
//! `[len: u32][bytes]`; numbers are big-endian. Binary, not JSON lines: the
//! helper stays dependency-free (a few hundred KB on the host), and a file's
//! bytes need no escaping.

/// The protocol version both ends must share (`Hello`).
pub const PROTOCOL: u32 = 1;

/// Largest message accepted; a longer length means a broken stream.
pub const MAX_MESSAGE: usize = 40 << 20;

/// Largest `Read` answered at once: a bigger file is read in ranges.
pub const MAX_READ: u32 = 4 << 20;

/// Largest file `Write` takes in one message.
pub const MAX_WRITE: usize = 32 << 20;

/// What a request asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    /// The handshake: answered with [`Reply::Hello`].
    Hello,
    Stat {
        path: String,
    },
    /// Up to `limit` entries of a directory from `offset`, sorted by name.
    List {
        path: String,
        offset: u32,
        limit: u32,
    },
    /// Up to `len` bytes from `offset` (at most [`MAX_READ`]).
    Read {
        path: String,
        offset: u64,
        len: u32,
    },
    /// Replace the file with `data`, atomically: a temp file beside it,
    /// synced, then renamed over it, keeping an existing file's mode.
    Write {
        path: String,
        data: Vec<u8>,
    },
    Mkdir {
        path: String,
        parents: bool,
    },
    Rename {
        from: String,
        to: String,
    },
    Delete {
        path: String,
        recursive: bool,
    },
}

/// What kind of thing an entry is (a symlink is reported as what it points
/// to, with [`Entry::symlink`] set).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    File = 1,
    Dir = 2,
    Other = 3,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The file name (the last part of the path); lossy if not UTF-8.
    pub name: String,
    pub kind: Kind,
    pub size: u64,
    /// Last modified, in milliseconds since the Unix epoch (0 if unknown).
    pub mtime_ms: i64,
    /// Unix permission bits (0 where there are none).
    pub mode: u32,
    pub symlink: bool,
}

/// Why a request failed, for srv to report the way a local error would be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrKind {
    NotFound = 1,
    PermissionDenied = 2,
    AlreadyExists = 3,
    NotADirectory = 4,
    IsADirectory = 5,
    DirectoryNotEmpty = 6,
    TooLarge = 7,
    Invalid = 8,
    Other = 255,
}

impl ErrKind {
    pub fn from_io(e: &std::io::Error) -> Self {
        use std::io::ErrorKind as K;
        match e.kind() {
            K::NotFound => ErrKind::NotFound,
            K::PermissionDenied => ErrKind::PermissionDenied,
            K::AlreadyExists => ErrKind::AlreadyExists,
            K::InvalidInput | K::InvalidData => ErrKind::Invalid,
            _ => match e.raw_os_error() {
                // ENOTDIR, EISDIR, ENOTEMPTY on Linux and macOS.
                Some(20) => ErrKind::NotADirectory,
                Some(21) => ErrKind::IsADirectory,
                Some(39) | Some(66) => ErrKind::DirectoryNotEmpty,
                _ => ErrKind::Other,
            },
        }
    }

    fn from_u8(b: u8) -> Self {
        match b {
            1 => ErrKind::NotFound,
            2 => ErrKind::PermissionDenied,
            3 => ErrKind::AlreadyExists,
            4 => ErrKind::NotADirectory,
            5 => ErrKind::IsADirectory,
            6 => ErrKind::DirectoryNotEmpty,
            7 => ErrKind::TooLarge,
            8 => ErrKind::Invalid,
            _ => ErrKind::Other,
        }
    }
}

/// The answer to one request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    Hello {
        protocol: u32,
        /// The user's home directory on the host (`~` in paths).
        home: String,
        /// `std::env::consts::OS` there: `linux`, `macos`, `windows`.
        os: String,
    },
    Stat(Entry),
    List {
        entries: Vec<Entry>,
        /// How many entries the directory has in all.
        total: u32,
    },
    Read {
        data: Vec<u8>,
        /// The file ends at or before this range's end.
        eof: bool,
    },
    Done,
    Err {
        kind: ErrKind,
        message: String,
    },
}

mod op {
    pub const HELLO: u8 = 1;
    pub const STAT: u8 = 2;
    pub const LIST: u8 = 3;
    pub const READ: u8 = 4;
    pub const WRITE: u8 = 5;
    pub const MKDIR: u8 = 6;
    pub const RENAME: u8 = 7;
    pub const DELETE: u8 = 8;
}

mod status {
    pub const HELLO: u8 = 1;
    pub const STAT: u8 = 2;
    pub const LIST: u8 = 3;
    pub const READ: u8 = 4;
    pub const DONE: u8 = 5;
    pub const ERR: u8 = 255;
}

/// Builds a body.
struct W(Vec<u8>);

impl W {
    fn u8(&mut self, v: u8) -> &mut Self {
        self.0.push(v);
        self
    }
    fn u32(&mut self, v: u32) -> &mut Self {
        self.0.extend_from_slice(&v.to_be_bytes());
        self
    }
    fn u64(&mut self, v: u64) -> &mut Self {
        self.0.extend_from_slice(&v.to_be_bytes());
        self
    }
    fn i64(&mut self, v: i64) -> &mut Self {
        self.0.extend_from_slice(&v.to_be_bytes());
        self
    }
    fn bytes(&mut self, v: &[u8]) -> &mut Self {
        self.u32(v.len() as u32);
        self.0.extend_from_slice(v);
        self
    }
    fn str(&mut self, v: &str) -> &mut Self {
        self.bytes(v.as_bytes())
    }
    fn entry(&mut self, e: &Entry) -> &mut Self {
        self.str(&e.name)
            .u8(e.kind as u8)
            .u64(e.size)
            .i64(e.mtime_ms)
            .u32(e.mode)
            .u8(u8::from(e.symlink))
    }
    /// The message: length, then body.
    fn message(self) -> Vec<u8> {
        let mut out = Vec::with_capacity(4 + self.0.len());
        out.extend_from_slice(&(self.0.len() as u32).to_be_bytes());
        out.extend_from_slice(&self.0);
        out
    }
}

/// Reads a body.
struct R<'a> {
    p: &'a [u8],
    i: usize,
}

impl<'a> R<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self.i.checked_add(n).filter(|&e| e <= self.p.len());
        let end = end.ok_or("message too short")?;
        let s = &self.p[self.i..end];
        self.i = end;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }
    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64, String> {
        Ok(u64::from_be_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn i64(&mut self) -> Result<i64, String> {
        Ok(i64::from_be_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn bytes(&mut self) -> Result<Vec<u8>, String> {
        let n = self.u32()? as usize;
        Ok(self.take(n)?.to_vec())
    }
    fn str(&mut self) -> Result<String, String> {
        String::from_utf8(self.bytes()?).map_err(|_| "a string is not UTF-8".to_string())
    }
    fn entry(&mut self) -> Result<Entry, String> {
        Ok(Entry {
            name: self.str()?,
            kind: match self.u8()? {
                1 => Kind::File,
                2 => Kind::Dir,
                _ => Kind::Other,
            },
            size: self.u64()?,
            mtime_ms: self.i64()?,
            mode: self.u32()?,
            symlink: self.u8()? != 0,
        })
    }
    fn done(&self) -> Result<(), String> {
        if self.i == self.p.len() {
            Ok(())
        } else {
            Err("trailing bytes in a message".into())
        }
    }
}

impl Request {
    /// The message for this request, with `id`.
    pub fn encode(&self, id: u32) -> Vec<u8> {
        let mut w = W(Vec::new());
        w.u32(id);
        match self {
            Request::Hello => {
                w.u8(op::HELLO);
            }
            Request::Stat { path } => {
                w.u8(op::STAT).str(path);
            }
            Request::List {
                path,
                offset,
                limit,
            } => {
                w.u8(op::LIST).str(path).u32(*offset).u32(*limit);
            }
            Request::Read { path, offset, len } => {
                w.u8(op::READ).str(path).u64(*offset).u32(*len);
            }
            Request::Write { path, data } => {
                w.u8(op::WRITE).str(path).bytes(data);
            }
            Request::Mkdir { path, parents } => {
                w.u8(op::MKDIR).str(path).u8(u8::from(*parents));
            }
            Request::Rename { from, to } => {
                w.u8(op::RENAME).str(from).str(to);
            }
            Request::Delete { path, recursive } => {
                w.u8(op::DELETE).str(path).u8(u8::from(*recursive));
            }
        }
        w.message()
    }

    /// A request body: its id and the request.
    pub fn decode(body: &[u8]) -> Result<(u32, Request), String> {
        let mut r = R { p: body, i: 0 };
        let id = r.u32()?;
        let req = match r.u8()? {
            op::HELLO => Request::Hello,
            op::STAT => Request::Stat { path: r.str()? },
            op::LIST => Request::List {
                path: r.str()?,
                offset: r.u32()?,
                limit: r.u32()?,
            },
            op::READ => Request::Read {
                path: r.str()?,
                offset: r.u64()?,
                len: r.u32()?,
            },
            op::WRITE => Request::Write {
                path: r.str()?,
                data: r.bytes()?,
            },
            op::MKDIR => Request::Mkdir {
                path: r.str()?,
                parents: r.u8()? != 0,
            },
            op::RENAME => Request::Rename {
                from: r.str()?,
                to: r.str()?,
            },
            op::DELETE => Request::Delete {
                path: r.str()?,
                recursive: r.u8()? != 0,
            },
            other => return Err(format!("unknown request {other}")),
        };
        r.done()?;
        Ok((id, req))
    }
}

impl Reply {
    /// The message for the reply to request `id`.
    pub fn encode(&self, id: u32) -> Vec<u8> {
        let mut w = W(Vec::new());
        w.u32(id);
        match self {
            Reply::Hello { protocol, home, os } => {
                w.u8(status::HELLO).u32(*protocol).str(home).str(os);
            }
            Reply::Stat(e) => {
                w.u8(status::STAT).entry(e);
            }
            Reply::List { entries, total } => {
                w.u8(status::LIST).u32(*total).u32(entries.len() as u32);
                for e in entries {
                    w.entry(e);
                }
            }
            Reply::Read { data, eof } => {
                w.u8(status::READ).u8(u8::from(*eof)).bytes(data);
            }
            Reply::Done => {
                w.u8(status::DONE);
            }
            Reply::Err { kind, message } => {
                w.u8(status::ERR).u8(*kind as u8).str(message);
            }
        }
        w.message()
    }

    /// A reply body: its request's id and the reply.
    pub fn decode(body: &[u8]) -> Result<(u32, Reply), String> {
        let mut r = R { p: body, i: 0 };
        let id = r.u32()?;
        let reply = match r.u8()? {
            status::HELLO => Reply::Hello {
                protocol: r.u32()?,
                home: r.str()?,
                os: r.str()?,
            },
            status::STAT => Reply::Stat(r.entry()?),
            status::LIST => {
                let total = r.u32()?;
                let n = r.u32()? as usize;
                // Each entry is at least 26 bytes: a count past that is a lie.
                if n > (body.len() / 26) {
                    return Err("list: impossible entry count".into());
                }
                let mut entries = Vec::with_capacity(n);
                for _ in 0..n {
                    entries.push(r.entry()?);
                }
                Reply::List { entries, total }
            }
            status::READ => Reply::Read {
                eof: r.u8()? != 0,
                data: r.bytes()?,
            },
            status::DONE => Reply::Done,
            status::ERR => Reply::Err {
                kind: ErrKind::from_u8(r.u8()?),
                message: r.str()?,
            },
            other => return Err(format!("unknown reply {other}")),
        };
        r.done()?;
        Ok((id, reply))
    }
}

/// Splits a byte stream that arrives in arbitrary pieces into message bodies.
#[derive(Debug, Default)]
pub struct Splitter {
    buf: Vec<u8>,
}

impl Splitter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add bytes; return every body now complete. `Err` for an impossible
    /// length: the stream is broken and the connection should be dropped.
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<Vec<u8>>, String> {
        self.buf.extend_from_slice(bytes);
        let mut bodies = Vec::new();
        let mut at = 0;
        while self.buf.len() - at >= 4 {
            let len = u32::from_be_bytes(self.buf[at..at + 4].try_into().unwrap()) as usize;
            if len > MAX_MESSAGE {
                return Err(format!("a message of {len} bytes is over the limit"));
            }
            if self.buf.len() - at - 4 < len {
                break;
            }
            bodies.push(self.buf[at + 4..at + 4 + len].to_vec());
            at += 4 + len;
        }
        self.buf.drain(..at);
        Ok(bodies)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str) -> Entry {
        Entry {
            name: name.into(),
            kind: Kind::File,
            size: 42,
            mtime_ms: 1_700_000_000_000,
            mode: 0o644,
            symlink: false,
        }
    }

    #[test]
    fn every_request_and_reply_survives_the_wire_in_any_pieces() {
        let requests = vec![
            Request::Hello,
            Request::Stat {
                path: "~/a b".into(),
            },
            Request::List {
                path: "/etc".into(),
                offset: 100,
                limit: 50,
            },
            Request::Read {
                path: "/x".into(),
                offset: 1 << 40,
                len: 4096,
            },
            Request::Write {
                path: "/x".into(),
                data: vec![0, 1, 2, 255],
            },
            Request::Mkdir {
                path: "/d".into(),
                parents: true,
            },
            Request::Rename {
                from: "/a".into(),
                to: "/b".into(),
            },
            Request::Delete {
                path: "/d".into(),
                recursive: false,
            },
        ];
        let replies = vec![
            Reply::Hello {
                protocol: PROTOCOL,
                home: "/home/u".into(),
                os: "linux".into(),
            },
            Reply::Stat(entry("a")),
            Reply::List {
                entries: vec![entry("a"), entry("ü")],
                total: 7,
            },
            Reply::Read {
                data: b"hello".to_vec(),
                eof: true,
            },
            Reply::Done,
            Reply::Err {
                kind: ErrKind::NotFound,
                message: "no such file".into(),
            },
        ];
        let mut stream = Vec::new();
        for (i, r) in requests.iter().enumerate() {
            stream.extend(r.encode(i as u32));
        }
        let mut s = Splitter::new();
        let mut bodies = Vec::new();
        for piece in stream.chunks(3) {
            bodies.extend(s.push(piece).unwrap());
        }
        let got: Vec<(u32, Request)> = bodies.iter().map(|b| Request::decode(b).unwrap()).collect();
        assert_eq!(
            got,
            requests
                .into_iter()
                .enumerate()
                .map(|(i, r)| (i as u32, r))
                .collect::<Vec<_>>()
        );

        for (i, r) in replies.iter().enumerate() {
            let msg = r.encode(i as u32 + 9);
            let (id, back) = Reply::decode(&msg[4..]).unwrap();
            assert_eq!((id, &back), (i as u32 + 9, r));
        }
    }

    #[test]
    fn a_broken_stream_is_refused_not_guessed_at() {
        assert!(Splitter::new().push(&u32::MAX.to_be_bytes()).is_err());
        assert!(Request::decode(&[0, 0, 0, 1, 99]).is_err());
        assert!(Request::decode(&[0, 0, 0, 1]).is_err());
        // A string longer than the message.
        assert!(Request::decode(&[0, 0, 0, 1, 2, 0, 0, 0, 9, b'a']).is_err());
        // Trailing bytes.
        assert!(Request::decode(&[0, 0, 0, 1, 1, 7]).is_err());
        // A list claiming more entries than could fit.
        let mut body = vec![0, 0, 0, 1, 3];
        body.extend_from_slice(&5u32.to_be_bytes());
        body.extend_from_slice(&1_000_000u32.to_be_bytes());
        assert!(Reply::decode(&body).is_err());
    }
}
