// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The session daemon (spec §7.2): one per remote user, listening on a Unix
//! socket in a `0700` directory, so only that user can connect, as with tmux.
//! It keeps each session's terminal and output history; `attach` connects to
//! it, and srv's frames go straight through to it.
//!
//! A connection starts with one line, then frames (`frame.rs`):
//!
//! - `AMR <protocol> ATTACH <session> <offset> <cols> <rows>`: attach to the
//!   session (created with a login shell if it does not exist), replaying its
//!   output from `offset`, then live. One client at a time: a new attach takes
//!   the session over and the old connection is closed.
//! - `AMR <protocol> END <session>`: end it. Answered `ok` or `none`.
//! - `AMR <protocol> LIST`: one line per session, `id end exited`.

use std::collections::HashMap;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::frame::{Decoder, Frame, PROTOCOL};
use crate::pty;
use crate::ring::{Ring, DEFAULT_CAPACITY};

/// Largest output frame sent in one piece when replaying.
const REPLAY_CHUNK: usize = 256 * 1024;

/// With no session and no connection for this long, the daemon exits.
const IDLE_EXIT: Duration = Duration::from_secs(60);

/// A client that cannot take a frame for this long is dropped: a stalled link
/// must not hold the session's lock (and so every reattach) behind it.
const CLIENT_WRITE_TIMEOUT: Duration = Duration::from_secs(3);

/// A session whose shell exited with no client attached keeps its last output
/// this long for a reattach to replay, then goes.
const EXITED_TTL: Duration = Duration::from_secs(10 * 60);

/// `<base>/run`, the daemon's private directory, and its socket.
pub fn run_dir(base: &Path) -> PathBuf {
    base.join("run")
}

pub fn socket_path(base: &Path) -> PathBuf {
    run_dir(base).join("daemon.sock")
}

/// A session id is AgentMux's: letters, digits, `-` and `_`, up to 64.
pub fn valid_session_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

struct Inner {
    ring: Ring,
    /// The attached connection, and its generation: frames are written only
    /// under this lock, so output, replay and replies never interleave.
    client: Option<(u64, UnixStream)>,
    exited: Option<i32>,
}

struct Session {
    id: String,
    master: File,
    pid: u32,
    inner: Mutex<Inner>,
}

type Sessions = Arc<Mutex<HashMap<String, Arc<Session>>>>;

static GENERATION: AtomicU64 = AtomicU64::new(1);

/// Run the daemon in the foreground until it has been idle for a minute.
pub fn run(base: &Path) -> io::Result<()> {
    let dir = run_dir(base);
    std::fs::create_dir_all(&dir)?;
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
    let sock = socket_path(base);
    // One daemon per user: whoever holds this lock owns the socket. Several
    // attaches starting daemons at once all race here; the losers exit and
    // their attaches connect to the winner. Held (open) for the daemon's life.
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join("daemon.lock"))?;
    // SAFETY: flock on a descriptor this process owns.
    if unsafe {
        libc::flock(
            std::os::fd::AsRawFd::as_raw_fd(&lock),
            libc::LOCK_EX | libc::LOCK_NB,
        )
    } != 0
    {
        return Ok(()); // another daemon is running or starting
    }
    let _lock = lock;
    let _ = std::fs::remove_file(&sock);
    let listener = UnixListener::bind(&sock)?;
    std::fs::set_permissions(&sock, std::fs::Permissions::from_mode(0o600))?;

    let sessions: Sessions = Arc::new(Mutex::new(HashMap::new()));
    let connections = Arc::new(AtomicUsize::new(0));
    {
        let (sessions, connections, sock) = (sessions.clone(), connections.clone(), sock.clone());
        std::thread::spawn(move || {
            let mut idle_since = Instant::now();
            loop {
                std::thread::sleep(Duration::from_secs(5));
                let busy =
                    !sessions.lock().unwrap().is_empty() || connections.load(Ordering::SeqCst) > 0;
                if busy {
                    idle_since = Instant::now();
                } else if idle_since.elapsed() >= IDLE_EXIT {
                    let _ = std::fs::remove_file(&sock);
                    std::process::exit(0);
                }
            }
        });
    }
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let (sessions, connections) = (sessions.clone(), connections.clone());
        connections.fetch_add(1, Ordering::SeqCst);
        std::thread::spawn(move || {
            let _ = handle(stream, &sessions);
            connections.fetch_sub(1, Ordering::SeqCst);
        });
    }
    Ok(())
}

fn handle(stream: UnixStream, sessions: &Sessions) -> io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let words: Vec<&str> = line.split_whitespace().collect();
    let mut out = stream.try_clone()?;
    match words.as_slice() {
        ["AMR", proto, rest @ ..] if proto.parse::<u32>().ok() == Some(PROTOCOL) => match rest {
            ["ATTACH", id, offset, cols, rows] if valid_session_id(id) => {
                let Ok(offset) = offset.parse::<u64>() else {
                    return out.write_all(&Frame::Error(format!("bad offset {offset:?}")).encode());
                };
                let (cols, rows) = (
                    cols.parse::<u16>().unwrap_or(80),
                    rows.parse::<u16>().unwrap_or(24),
                );
                attach(stream, reader, sessions, id, offset, cols, rows)
            }
            ["END", id] => {
                let found = end_session(sessions, id);
                out.write_all(if found { b"ok\n" } else { b"none\n" })
            }
            ["LIST"] => {
                // The sessions first, then each one's state: never the map's
                // lock while waiting on a session's.
                let all: Vec<Arc<Session>> = sessions.lock().unwrap().values().cloned().collect();
                let list: Vec<String> = all
                    .iter()
                    .map(|s| {
                        let inner = s.inner.lock().unwrap();
                        let exited = inner
                            .exited
                            .map(|c| c.to_string())
                            .unwrap_or_else(|| "-".into());
                        format!("{} {} {}\n", s.id, inner.ring.end(), exited)
                    })
                    .collect();
                out.write_all(list.concat().as_bytes())
            }
            _ => out.write_all(&Frame::Error("bad request".into()).encode()),
        },
        _ => {
            out.write_all(&Frame::Error(format!("this helper speaks protocol {PROTOCOL}")).encode())
        }
    }
}

fn end_session(sessions: &Sessions, id: &str) -> bool {
    let Some(s) = sessions.lock().unwrap().remove(id) else {
        return false;
    };
    let mut inner = s.inner.lock().unwrap();
    // A shell already reaped has no process group left to end, and its pid
    // may by now be another process's.
    if inner.exited.is_none() {
        pty::end_group(s.pid);
    }
    if let Some((_, c)) = inner.client.take() {
        let _ = c.shutdown(std::net::Shutdown::Both);
    }
    true
}

fn attach(
    stream: UnixStream,
    mut reader: BufReader<UnixStream>,
    sessions: &Sessions,
    id: &str,
    offset: u64,
    cols: u16,
    rows: u16,
) -> io::Result<()> {
    let (session, created) = {
        let mut map = sessions.lock().unwrap();
        match map.get(id) {
            Some(s) => (s.clone(), false),
            None => {
                let p = match pty::spawn_login_shell(cols, rows) {
                    Ok(p) => p,
                    Err(e) => {
                        let mut out = stream;
                        return out.write_all(
                            &Frame::Error(format!("could not start a shell: {e}")).encode(),
                        );
                    }
                };
                let s = Arc::new(Session {
                    id: id.to_string(),
                    master: p.master,
                    pid: p.child.id(),
                    inner: Mutex::new(Inner {
                        ring: Ring::new(DEFAULT_CAPACITY),
                        client: None,
                        exited: None,
                    }),
                });
                start_reader(s.clone(), p.child, sessions.clone());
                map.insert(id.to_string(), s.clone());
                (s, true)
            }
        }
    };
    if !created {
        pty::resize(&session.master, cols, rows);
    }

    // Replay and take over, under the lock the reader appends under: nothing
    // between the replay and the live stream is lost or sent twice.
    let generation = GENERATION.fetch_add(1, Ordering::SeqCst);
    stream.set_write_timeout(Some(CLIENT_WRITE_TIMEOUT))?;
    {
        let mut inner = session.inner.lock().unwrap();
        let mut out = stream.try_clone()?;
        out.write_all(
            &Frame::Hello {
                session: id.to_string(),
                created,
                end: inner.ring.end(),
            }
            .encode(),
        )?;
        let replay = inner.ring.read_from(offset);
        if let Some((from, to)) = replay.lost {
            out.write_all(&Frame::Lost { from, to }.encode())?;
        }
        let mut at = replay.offset;
        for chunk in replay.data.chunks(REPLAY_CHUNK) {
            out.write_all(
                &Frame::Output {
                    offset: at,
                    data: chunk.to_vec(),
                }
                .encode(),
            )?;
            at += chunk.len() as u64;
        }
        if let Some(code) = inner.exited {
            out.write_all(&Frame::Exited { code }.encode())?;
            drop(inner);
            sessions.lock().unwrap().remove(id);
            return Ok(());
        }
        if let Some((_, old)) = inner.client.replace((generation, out)) {
            let _ = old.shutdown(std::net::Shutdown::Both);
        }
    }

    // Frames from srv.
    let mut decoder = Decoder::new();
    let mut buf = [0u8; 16 * 1024];
    let mut master = session.master.try_clone()?;
    loop {
        let n = match reader.read(&mut buf) {
            Ok(0) | Err(_) => break, // the link dropped: detach, the session stays
            Ok(n) => n,
        };
        let Ok(frames) = decoder.push(&buf[..n]) else {
            break;
        };
        for frame in frames {
            match frame {
                Frame::Input(data) => {
                    let _ = master.write_all(&data);
                }
                Frame::Resize { cols, rows } => pty::resize(&session.master, cols, rows),
                Frame::Ping => {
                    let mut inner = session.inner.lock().unwrap();
                    if let Some((g, c)) = inner.client.as_mut() {
                        if *g == generation {
                            let _ = c.write_all(&Frame::Pong.encode());
                        }
                    }
                }
                Frame::Detach => {
                    release(&session, generation);
                    return Ok(());
                }
                Frame::End => {
                    end_session(sessions, id);
                    return Ok(());
                }
                _ => {}
            }
        }
    }
    release(&session, generation);
    Ok(())
}

/// Drop this connection as the session's client, if it still is.
fn release(session: &Session, generation: u64) {
    let mut inner = session.inner.lock().unwrap();
    if inner.client.as_ref().is_some_and(|(g, _)| *g == generation) {
        inner.client = None;
    }
}

/// Read the session's terminal into its ring and to its client, until the
/// shell exits.
fn start_reader(session: Arc<Session>, mut child: std::process::Child, sessions: Sessions) {
    std::thread::spawn(move || {
        let mut master = match session.master.try_clone() {
            Ok(m) => m,
            Err(_) => return,
        };
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            match master.read(&mut buf) {
                Ok(0) | Err(_) => break, // EOF or EIO: the terminal closed
                Ok(n) => {
                    let mut inner = session.inner.lock().unwrap();
                    let offset = inner.ring.end();
                    inner.ring.append(&buf[..n]);
                    let frame = Frame::Output {
                        offset,
                        data: buf[..n].to_vec(),
                    }
                    .encode();
                    if let Some((_, c)) = inner.client.as_mut() {
                        if c.write_all(&frame).is_err() {
                            inner.client = None;
                        }
                    }
                }
            }
        }
        let code = child.wait().ok().and_then(|s| s.code()).unwrap_or(-1);
        let mut inner = session.inner.lock().unwrap();
        inner.exited = Some(code);
        // An attached client hears it and the session goes; otherwise it waits
        // for the next attach to replay its last output and the exit.
        if let Some((_, mut c)) = inner.client.take() {
            let _ = c.write_all(&Frame::Exited { code }.encode());
            let _ = c.shutdown(std::net::Shutdown::Both);
            drop(inner);
            sessions.lock().unwrap().remove(&session.id);
        } else {
            drop(inner);
            std::thread::sleep(EXITED_TTL);
            let mut map = sessions.lock().unwrap();
            if map
                .get(&session.id)
                .is_some_and(|s| Arc::ptr_eq(s, &session))
            {
                map.remove(&session.id);
            }
        }
    });
}
