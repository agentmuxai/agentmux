// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Files on an SSH host, through AgentMux's helper (spec §6.3 of
//! SPEC_REMOTE_TERMINALS_AND_DURABLE_SESSIONS_2026_10_02.md): one
//! `ssh -T host -- agentmux-remote serve --stdio` per connection, kept open
//! while it is used and closed after [`IDLE_CLOSE`] without a call, speaking
//! `agentmux_remote::fsproto`. Requests carry ids, so any number can be in
//! flight on one connection.
//!
//! A host without this version's helper gets it installed on first use
//! (`helper_install::ensure`), as a durable pane does. ssh's prompts go to
//! the user in the window of the pane that asked ([`connect`]'s `ask`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use agentmux_remote::fsproto::{self, Entry, ErrKind, Reply, Request, Splitter};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::{mpsc, oneshot};

use super::host::HostSsh;
use super::sessions::AskIn;

/// A connection nobody has used for this long is closed.
pub const IDLE_CLOSE: Duration = Duration::from_secs(10 * 60);

/// How long the handshake may take: ssh may first ask the user something
/// (askpass, whose dialog waits up to two minutes).
const HANDSHAKE: Duration = Duration::from_secs(150);

/// How long one request may take once connected.
const CALL: Duration = Duration::from_secs(120);

/// Why a remote file operation failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteError {
    /// What the host said went wrong; `None` when the connection itself
    /// failed (it could not be made, it dropped, or a request timed out).
    pub kind: Option<ErrKind>,
    pub message: String,
}

impl RemoteError {
    fn link(message: impl Into<String>) -> Self {
        Self {
            kind: None,
            message: message.into(),
        }
    }

    pub fn is_not_found(&self) -> bool {
        self.kind == Some(ErrKind::NotFound)
    }
}

impl std::fmt::Display for RemoteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

struct Call {
    req: Request,
    reply: oneshot::Sender<Result<Reply, String>>,
}

/// One open connection to a host's helper.
pub struct RemoteFiles {
    /// The connection's canonical name.
    pub conn: String,
    /// The user's home directory there (`~`).
    pub home: String,
    /// `std::env::consts::OS` there.
    pub os: String,
    calls: mpsc::UnboundedSender<Call>,
    last_used: Mutex<Instant>,
}

impl RemoteFiles {
    /// Speak the protocol over `reader` and `writer` (an ssh's stdout and
    /// stdin), handshake included. `keep` lives as long as the connection
    /// (the ssh child and its askpass grant).
    pub async fn over<R, W>(
        conn: &str,
        reader: R,
        writer: W,
        keep: Box<dyn std::any::Any + Send>,
    ) -> Result<Arc<Self>, RemoteError>
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        let (tx, rx) = mpsc::unbounded_channel();
        tokio::spawn(pump(conn.to_string(), reader, writer, rx, keep));
        let hello = call(&tx, Request::Hello, HANDSHAKE).await?;
        let Reply::Hello { protocol, home, os } = hello else {
            return Err(RemoteError::link(format!(
                "the helper on {conn} did not greet"
            )));
        };
        if protocol != fsproto::PROTOCOL {
            return Err(RemoteError::link(format!(
                "the helper on {conn} speaks file protocol {protocol}, this AgentMux {}",
                fsproto::PROTOCOL
            )));
        }
        Ok(Arc::new(Self {
            conn: conn.to_string(),
            home,
            os,
            calls: tx,
            last_used: Mutex::new(Instant::now()),
        }))
    }

    /// Whether the connection is still up.
    pub fn is_open(&self) -> bool {
        !self.calls.is_closed()
    }

    /// One request; a host's refusal is `Err` with its kind.
    pub async fn call(&self, req: Request) -> Result<Reply, RemoteError> {
        *self.last_used.lock().unwrap() = Instant::now();
        match call(&self.calls, req, CALL).await? {
            Reply::Err { kind, message } => Err(RemoteError {
                kind: Some(kind),
                message,
            }),
            reply => Ok(reply),
        }
    }

    pub async fn stat(&self, path: &str) -> Result<Entry, RemoteError> {
        match self.call(Request::Stat { path: path.into() }).await? {
            Reply::Stat(e) => Ok(e),
            r => Err(unexpected(&r)),
        }
    }

    /// Every entry of a directory, a page at a time.
    pub async fn list(&self, path: &str) -> Result<Vec<Entry>, RemoteError> {
        let mut all = Vec::new();
        loop {
            let req = Request::List {
                path: path.into(),
                offset: all.len() as u32,
                limit: 1000,
            };
            match self.call(req).await? {
                Reply::List { entries, total } => {
                    let done = entries.is_empty() || all.len() + entries.len() >= total as usize;
                    all.extend(entries);
                    if done {
                        return Ok(all);
                    }
                }
                r => return Err(unexpected(&r)),
            }
        }
    }

    /// A whole file of at most `max` bytes, read in ranges.
    pub async fn read(&self, path: &str, max: u64) -> Result<Vec<u8>, RemoteError> {
        let size = self.stat(path).await?.size;
        if size > max {
            return Err(RemoteError {
                kind: Some(ErrKind::TooLarge),
                message: format!("{path} is {size} bytes, over the {max} this can open"),
            });
        }
        let mut data = Vec::with_capacity(size as usize);
        loop {
            let req = Request::Read {
                path: path.into(),
                offset: data.len() as u64,
                len: fsproto::MAX_READ,
            };
            match self.call(req).await? {
                Reply::Read { data: part, eof } => {
                    data.extend_from_slice(&part);
                    if data.len() as u64 > max {
                        return Err(RemoteError {
                            kind: Some(ErrKind::TooLarge),
                            message: format!("{path} grew past {max} bytes while it was read"),
                        });
                    }
                    if eof || part.is_empty() {
                        return Ok(data);
                    }
                }
                r => return Err(unexpected(&r)),
            }
        }
    }

    /// Replace a file with `data`, atomically on the host.
    pub async fn write(&self, path: &str, data: Vec<u8>) -> Result<(), RemoteError> {
        if data.len() > fsproto::MAX_WRITE {
            return Err(RemoteError {
                kind: Some(ErrKind::TooLarge),
                message: format!(
                    "over the {} MB a remote write may be",
                    fsproto::MAX_WRITE >> 20
                ),
            });
        }
        self.done(Request::Write {
            path: path.into(),
            data,
        })
        .await
    }

    /// Add `data` to the end of an existing file (a big upload, in pieces).
    pub async fn append(&self, path: &str, data: Vec<u8>) -> Result<(), RemoteError> {
        self.done(Request::Append {
            path: path.into(),
            data,
        })
        .await
    }

    /// One range of a file: up to `len` bytes from `offset`, and whether the
    /// file ends there.
    pub async fn read_range(
        &self,
        path: &str,
        offset: u64,
        len: u32,
    ) -> Result<(Vec<u8>, bool), RemoteError> {
        match self
            .call(Request::Read {
                path: path.into(),
                offset,
                len: len.min(fsproto::MAX_READ),
            })
            .await?
        {
            Reply::Read { data, eof } => Ok((data, eof)),
            r => Err(unexpected(&r)),
        }
    }

    /// Rename the file `from` over the file `to` in one step.
    pub async fn replace(&self, from: &str, to: &str) -> Result<(), RemoteError> {
        self.done(Request::Replace {
            from: from.into(),
            to: to.into(),
        })
        .await
    }

    /// `path` with its links and `..` resolved.
    pub async fn realpath(&self, path: &str) -> Result<String, RemoteError> {
        match self.call(Request::Realpath { path: path.into() }).await? {
            Reply::Path(p) => Ok(p),
            r => Err(unexpected(&r)),
        }
    }

    /// A file's permission bits (`mode`, 0 to leave) and modification time
    /// (`mtime_ms`, 0 to leave).
    pub async fn set_meta(&self, path: &str, mode: u32, mtime_ms: i64) -> Result<(), RemoteError> {
        self.done(Request::SetMeta {
            path: path.into(),
            mode,
            mtime_ms,
        })
        .await
    }

    /// Flush a file's contents to disk.
    pub async fn sync(&self, path: &str) -> Result<(), RemoteError> {
        self.done(Request::Sync { path: path.into() }).await
    }

    pub async fn mkdir(&self, path: &str, parents: bool) -> Result<(), RemoteError> {
        self.done(Request::Mkdir {
            path: path.into(),
            parents,
        })
        .await
    }

    pub async fn rename(&self, from: &str, to: &str) -> Result<(), RemoteError> {
        self.done(Request::Rename {
            from: from.into(),
            to: to.into(),
        })
        .await
    }

    pub async fn delete(&self, path: &str, recursive: bool) -> Result<(), RemoteError> {
        self.done(Request::Delete {
            path: path.into(),
            recursive,
        })
        .await
    }

    async fn done(&self, req: Request) -> Result<(), RemoteError> {
        match self.call(req).await? {
            Reply::Done => Ok(()),
            r => Err(unexpected(&r)),
        }
    }

    fn idle_for(&self) -> Duration {
        self.last_used.lock().unwrap().elapsed()
    }
}

fn unexpected(r: &Reply) -> RemoteError {
    RemoteError::link(format!("the helper answered out of turn: {r:?}"))
}

async fn call(
    calls: &mpsc::UnboundedSender<Call>,
    req: Request,
    limit: Duration,
) -> Result<Reply, RemoteError> {
    let (reply, rx) = oneshot::channel();
    calls
        .send(Call { req, reply })
        .map_err(|_| RemoteError::link("the connection to the host is closed"))?;
    match tokio::time::timeout(limit, rx).await {
        Ok(Ok(Ok(reply))) => Ok(reply),
        Ok(Ok(Err(e))) => Err(RemoteError::link(e)),
        Ok(Err(_)) => Err(RemoteError::link("the connection to the host closed")),
        Err(_) => Err(RemoteError::link("the host took too long to answer")),
    }
}

/// The connection's one task: requests out, replies back to whoever asked,
/// until every handle is gone or the stream ends; then every call still
/// waiting hears why, and `keep` (the ssh) is dropped.
async fn pump<R, W>(
    conn: String,
    mut reader: R,
    writer: W,
    mut calls: mpsc::UnboundedReceiver<Call>,
    keep: Box<dyn std::any::Any + Send>,
) where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin + Send + 'static,
{
    // Writes on their own task: replies are read while a big request (up to
    // 32 MB) is still going out. The helper answers one request at a time and
    // stops reading while it writes a reply, so a writer that waited here,
    // not reading, would leave both pipes full and both ends stuck.
    let (out_tx, out_rx) = mpsc::unbounded_channel::<Vec<u8>>();
    let (failed_tx, mut failed_rx) = oneshot::channel::<String>();
    tokio::spawn(write_out(conn.clone(), writer, out_rx, failed_tx));
    let mut waiting: HashMap<u32, oneshot::Sender<Result<Reply, String>>> = HashMap::new();
    let mut next_id = 0u32;
    let mut splitter = Splitter::new();
    let mut buf = vec![0u8; 64 * 1024];
    let why = loop {
        tokio::select! {
            c = calls.recv() => {
                let Some(c) = c else { break "closed".to_string() };
                next_id = next_id.wrapping_add(1);
                let id = next_id;
                if out_tx.send(c.req.encode(id)).is_err() {
                    let _ = c.reply.send(Err(format!("the connection to {conn} closed")));
                    break format!("the connection to {conn} closed");
                }
                waiting.insert(id, c.reply);
            }
            failed = &mut failed_rx => {
                break failed.unwrap_or_else(|_| format!("the connection to {conn} closed"));
            }
            n = reader.read(&mut buf) => {
                let n = match n {
                    Ok(0) => break format!("the connection to {conn} closed"),
                    Ok(n) => n,
                    Err(e) => break format!("the connection to {conn} dropped ({e})"),
                };
                let bodies = match splitter.push(&buf[..n]) {
                    Ok(b) => b,
                    Err(e) => break format!("the helper on {conn} sent something unreadable ({e})"),
                };
                for body in bodies {
                    match Reply::decode(&body) {
                        Ok((id, reply)) => {
                            if let Some(w) = waiting.remove(&id) {
                                let _ = w.send(Ok(reply));
                            }
                        }
                        Err(e) => {
                            tracing::warn!(conn = %conn, error = %e, "remote files: unreadable reply");
                        }
                    }
                }
            }
        }
    };
    for (_, w) in waiting.drain() {
        let _ = w.send(Err(why.clone()));
    }
    // Calls that arrived after the end hear it too.
    calls.close();
    while let Ok(c) = calls.try_recv() {
        let _ = c.reply.send(Err(why.clone()));
    }
    // Ends the writer (its queue closes), then the ssh.
    drop(out_tx);
    drop(keep);
}

/// The connection's writer: each request's bytes, in order. A failed write
/// is reported once, and ends the connection.
async fn write_out<W: AsyncWrite + Unpin>(
    conn: String,
    mut writer: W,
    mut out: mpsc::UnboundedReceiver<Vec<u8>>,
    failed: oneshot::Sender<String>,
) {
    while let Some(bytes) = out.recv().await {
        let result = match writer.write_all(&bytes).await {
            Ok(()) => writer.flush().await,
            Err(e) => Err(e),
        };
        if let Err(e) = result {
            let _ = failed.send(format!("the connection to {conn} dropped ({e})"));
            return;
        }
    }
}

/// One host's slot in the pool: a lock so two first calls make one
/// connection, around the connection once made.
type Slot = Arc<tokio::sync::Mutex<Option<Arc<RemoteFiles>>>>;

fn pool() -> &'static Mutex<HashMap<String, Slot>> {
    static POOL: OnceLock<Mutex<HashMap<String, Slot>>> = OnceLock::new();
    POOL.get_or_init(|| {
        // Close what nobody uses (once, for the whole pool).
        tokio::spawn(async {
            loop {
                tokio::time::sleep(Duration::from_secs(60)).await;
                close_idle(IDLE_CLOSE).await;
            }
        });
        Mutex::new(HashMap::new())
    })
}

/// Drop every connection idle for `after` or more (each ssh then ends).
pub async fn close_idle(after: Duration) {
    let slots: Vec<Slot> = pool().lock().unwrap().values().cloned().collect();
    for slot in slots {
        let mut s = slot.lock().await;
        if s.as_ref()
            .is_some_and(|f| !f.is_open() || f.idle_for() >= after)
        {
            *s = None;
        }
    }
}

/// The open connection to `conn`'s helper, made (and the helper installed)
/// if there is none. `ask`: where ssh's prompts go, if any are needed.
pub async fn connect(conn: &str, ask: Option<AskIn<'_>>) -> Result<Arc<RemoteFiles>, RemoteError> {
    let name = canonical(conn)?;
    let slot = pool()
        .lock()
        .unwrap()
        .entry(name.clone())
        .or_default()
        .clone();
    let mut s = slot.lock().await;
    if let Some(f) = s.as_ref().filter(|f| f.is_open()) {
        return Ok(f.clone());
    }
    let mut host = HostSsh::for_connection(conn).map_err(RemoteError::link)?;
    let files = match start(&name, host.clone(), ask).await {
        Ok(f) => f,
        // The helper is not there (or not this version): install it, once,
        // with ssh's prompts going to the user as for the connection itself
        // (the grant lives until the install is done).
        Err((e, true)) => {
            let _install_grant = ask.and_then(|a| host.ask_user_in(a.block_id, &name, a.auth_key));
            super::helper_install::ensure(&host, &name, "files", |_| async {})
                .await
                .map_err(|i| {
                    RemoteError::link(format!(
                        "{} (and installing the helper failed: {i})",
                        e.message
                    ))
                })?;
            start(&name, host, ask).await.map_err(|(e, _)| e)?
        }
        Err((e, false)) => return Err(e),
    };
    // The helper answered: the host has it (spec §7.1; Remotes shows it).
    super::helper_hosts::remember(&name);
    *s = Some(files.clone());
    Ok(files)
}

/// The pool's key for `conn`: its canonical name, one per host however it is
/// spelled. `Err` unless it is an SSH connection.
fn canonical(conn: &str) -> Result<String, RemoteError> {
    match super::ConnTarget::parse(conn).map_err(RemoteError::link)? {
        t @ super::ConnTarget::Ssh(_) => Ok(t.name()),
        _ => Err(RemoteError::link(format!(
            "{conn:?} is not an SSH connection"
        ))),
    }
}

/// Tests: a connection to `conn` that is the real helper's request handling
/// in this process (the files under `home`), put in the pool so [`connect`]
/// returns it as it would a connection over ssh.
#[cfg(test)]
pub(crate) async fn connect_in_process(conn: &str, home: std::path::PathBuf) -> Arc<RemoteFiles> {
    let (r, w) = testing::helper(home);
    let f = RemoteFiles::over(conn, r, w, Box::new(())).await.unwrap();
    let name = canonical(conn).unwrap();
    let slot = pool().lock().unwrap().entry(name).or_default().clone();
    *slot.lock().await = Some(f.clone());
    f
}

#[cfg(test)]
pub(crate) mod testing {
    use super::*;

    /// A helper in this process: the real `serve::handle` behind an
    /// in-memory pipe, as ssh would carry it.
    pub(crate) fn helper(
        home: std::path::PathBuf,
    ) -> (tokio::io::DuplexStream, tokio::io::DuplexStream) {
        helper_with_pipes(home, 1 << 20)
    }

    /// [`helper`] with pipes of `size` bytes each way (an ssh's are small).
    pub(crate) fn helper_with_pipes(
        home: std::path::PathBuf,
        size: usize,
    ) -> (tokio::io::DuplexStream, tokio::io::DuplexStream) {
        helper_with(home, size, Duration::ZERO)
    }

    /// [`helper`] that takes `delay` over each request, as a slow link
    /// would: long enough to cancel something halfway.
    pub(crate) fn slow_helper(
        home: std::path::PathBuf,
        delay: Duration,
    ) -> (tokio::io::DuplexStream, tokio::io::DuplexStream) {
        helper_with(home, 1 << 20, delay)
    }

    fn helper_with(
        home: std::path::PathBuf,
        size: usize,
        delay: Duration,
    ) -> (tokio::io::DuplexStream, tokio::io::DuplexStream) {
        let (client_read, mut helper_write) = tokio::io::duplex(size);
        let (mut helper_read, client_write) = tokio::io::duplex(size);
        tokio::spawn(async move {
            let mut s = Splitter::new();
            let mut buf = vec![0u8; 65536];
            loop {
                let n = match helper_read.read(&mut buf).await {
                    Ok(0) | Err(_) => return,
                    Ok(n) => n,
                };
                for body in s.push(&buf[..n]).unwrap() {
                    if !delay.is_zero() {
                        tokio::time::sleep(delay).await;
                    }
                    let (id, req) = Request::decode(&body).unwrap();
                    let reply = agentmux_remote::serve::handle(&home, req);
                    helper_write.write_all(&reply.encode(id)).await.unwrap();
                }
            }
        });
        (client_read, client_write)
    }
}

/// Start `serve --stdio` on the host. `Err((why, true))` when the helper
/// looks missing (the stream ended before the greeting), so installing it
/// is worth a try.
async fn start(
    name: &str,
    mut host: HostSsh,
    ask: Option<AskIn<'_>>,
) -> Result<Arc<RemoteFiles>, (RemoteError, bool)> {
    let grant = ask.and_then(|a| host.ask_user_in(a.block_id, name, a.auth_key));
    let remote = format!(
        "{} serve --stdio",
        crate::backend::blockcontroller::durable_ssh::helper_path()
    );
    let mut cmd = host.command(&remote);
    cmd.stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = cmd
        .spawn()
        .map_err(|e| (RemoteError::link(format!("could not run ssh: {e}")), false))?;
    let (Some(stdin), Some(stdout), Some(mut stderr)) =
        (child.stdin.take(), child.stdout.take(), child.stderr.take())
    else {
        return Err((RemoteError::link("ssh started without its pipes"), false));
    };
    // ssh's complaints, kept for the error if the greeting never comes.
    let said = Arc::new(Mutex::new(String::new()));
    {
        let said = said.clone();
        tokio::spawn(async move {
            let mut buf = Vec::new();
            let _ = stderr.read_to_end(&mut buf).await;
            *said.lock().unwrap() = String::from_utf8_lossy(&buf).into_owned();
        });
    }
    match RemoteFiles::over(name, stdout, stdin, Box::new((child, grant))).await {
        Ok(f) => Ok(f),
        Err(e) => {
            // Give stderr a moment to arrive after the stream ended.
            tokio::time::sleep(Duration::from_millis(200)).await;
            let said = said.lock().unwrap().clone();
            let last = said
                .lines()
                .rev()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("")
                .trim()
                .to_string();
            let missing = said.contains("not found") || said.contains("No such file");
            let message = if last.is_empty() {
                e.message
            } else {
                format!("{} ({last})", e.message)
            };
            Err((RemoteError::link(message), missing))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use super::testing::helper;

    #[tokio::test]
    async fn files_round_trip_through_the_helper() {
        let home = tempfile::tempdir().unwrap();
        let (r, w) = helper(home.path().to_path_buf());
        let f = RemoteFiles::over("testhost", r, w, Box::new(()))
            .await
            .unwrap();
        assert_eq!(std::path::Path::new(&f.home), home.path());

        f.mkdir("~/d", false).await.unwrap();
        // Bigger than one ranged read, so `read` has to take several.
        let big: Vec<u8> = (0..(fsproto::MAX_READ as usize + 1000))
            .map(|i| i as u8)
            .collect();
        f.write("~/d/big.bin", big.clone()).await.unwrap();
        assert_eq!(f.read("~/d/big.bin", 64 << 20).await.unwrap(), big);
        let e = f.read("~/d/big.bin", 10).await.unwrap_err();
        assert_eq!(e.kind, Some(ErrKind::TooLarge));

        for i in 0..1205 {
            f.write(&format!("~/d/f{i:04}"), vec![]).await.unwrap();
        }
        // Several pages, all of them.
        assert_eq!(f.list("~/d").await.unwrap().len(), 1206);

        f.rename("~/d/f0000", "~/d/renamed").await.unwrap();
        assert!(f.stat("~/d/f0000").await.unwrap_err().is_not_found());
        assert_eq!(f.stat("~/d/renamed").await.unwrap().size, 0);
        f.delete("~/d", true).await.unwrap();
        assert!(f.stat("~/d").await.unwrap_err().is_not_found());
    }

    /// A big read's reply and a big write's request at once, over pipes far
    /// smaller than either, against a helper that stops reading while it
    /// writes: both finish (a writer that blocked the reader would not).
    #[tokio::test]
    async fn a_big_write_and_a_big_read_at_once_do_not_jam_small_pipes() {
        let home = tempfile::tempdir().unwrap();
        let big: Vec<u8> = (0..(3 << 20)).map(|i| (i % 251) as u8).collect();
        std::fs::write(home.path().join("in.bin"), &big).unwrap();
        let (r, w) = super::testing::helper_with_pipes(home.path().to_path_buf(), 64 * 1024);
        let f = RemoteFiles::over("testhost", r, w, Box::new(()))
            .await
            .unwrap();
        let (read, wrote) = tokio::time::timeout(Duration::from_secs(30), async {
            tokio::join!(
                f.read("~/in.bin", 64 << 20),
                f.write("~/out.bin", vec![7u8; 8 << 20])
            )
        })
        .await
        .expect("no deadlock");
        assert_eq!(read.unwrap(), big);
        wrote.unwrap();
        assert_eq!(
            std::fs::metadata(home.path().join("out.bin"))
                .unwrap()
                .len(),
            8 << 20
        );
    }

    #[tokio::test]
    async fn many_calls_in_flight_each_get_their_own_answer() {
        let home = tempfile::tempdir().unwrap();
        for i in 0..50 {
            std::fs::write(home.path().join(format!("n{i}")), i.to_string()).unwrap();
        }
        let (r, w) = helper(home.path().to_path_buf());
        let f = RemoteFiles::over("testhost", r, w, Box::new(()))
            .await
            .unwrap();
        let reads = (0..50).map(|i| {
            let f = f.clone();
            async move { (i, f.read(&format!("~/n{i}"), 100).await.unwrap()) }
        });
        for (i, data) in futures_util::future::join_all(reads).await {
            assert_eq!(data, i.to_string().into_bytes());
        }
    }

    #[tokio::test]
    async fn a_dropped_link_fails_every_call_and_closes() {
        let (client_read, helper_write) = tokio::io::duplex(1024);
        let (mut helper_read, client_write) = tokio::io::duplex(1024);
        // Greets, then drops at the next request.
        let helper = tokio::spawn(async move {
            let mut helper_write = helper_write;
            let mut buf = vec![0u8; 1024];
            let n = helper_read.read(&mut buf).await.unwrap();
            let (id, _) = Request::decode(&Splitter::new().push(&buf[..n]).unwrap()[0]).unwrap();
            let hello = Reply::Hello {
                protocol: fsproto::PROTOCOL,
                home: "/home/u".into(),
                os: "linux".into(),
            };
            helper_write.write_all(&hello.encode(id)).await.unwrap();
            let _ = helper_read.read(&mut buf).await;
        });
        let f = RemoteFiles::over("testhost", client_read, client_write, Box::new(()))
            .await
            .unwrap();
        let pending = f.stat("~/x");
        let (r, _) = tokio::join!(pending, helper);
        let e = r.unwrap_err();
        assert_eq!(e.kind, None, "{e:?}");
        assert!(e.message.contains("closed"), "{e:?}");
        assert!(!f.is_open());
        assert!(f.stat("~/y").await.is_err());
    }

    #[tokio::test]
    async fn a_helper_of_another_protocol_is_refused() {
        let (client_read, mut helper_write) = tokio::io::duplex(1024);
        let (mut helper_read, client_write) = tokio::io::duplex(1024);
        tokio::spawn(async move {
            let mut buf = vec![0u8; 1024];
            let n = helper_read.read(&mut buf).await.unwrap();
            let (id, _) = Request::decode(&Splitter::new().push(&buf[..n]).unwrap()[0]).unwrap();
            let hello = Reply::Hello {
                protocol: fsproto::PROTOCOL + 1,
                home: "/".into(),
                os: "linux".into(),
            };
            helper_write.write_all(&hello.encode(id)).await.unwrap();
            let _ = helper_read.read(&mut buf).await;
        });
        let e = RemoteFiles::over("testhost", client_read, client_write, Box::new(()))
            .await
            .err()
            .unwrap();
        assert!(e.message.contains("file protocol"), "{e:?}");
    }
}
