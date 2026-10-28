// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The helper processes that call the operating system, and the way srv talks to them.
//!
//! The shell loads third-party code (icon handlers, codecs, cloud providers), so
//! none of it runs in srv: srv re-runs its own executable as a small helper
//! ([`super::CHILD_ARG`]), sends it one request at a time over its pipes and
//! kills it when it doesn't answer in time (spec §4.2). The helper dies with
//! srv: its stdin closes.
//!
//! Protocol, one request in flight per helper. Request: one JSON line,
//! `{"kind":"file","ext":"rs","px":16}`. Answer: one JSON line, either
//! `{"ok":true,"len":N}` followed by N bytes of PNG, or `{"ok":false,"err":"..."}`.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout};

use super::request::IconRequest;
use super::IconError;

/// How long one request may take before its helper is killed.
pub const DEADLINE: Duration = Duration::from_secs(3);
/// Helpers kept: one slow icon shouldn't hold up every other.
pub const WORKERS: usize = 2;
/// Requests allowed to wait for a helper before the rest are refused.
pub const QUEUE_MAX: usize = 64;
/// The most a picture may weigh. A 256 px RGBA PNG is far smaller.
pub const MAX_PNG_BYTES: usize = 1024 * 1024;
/// The longest answer line read.
const MAX_LINE: usize = 512;

#[derive(Serialize)]
pub struct WireRequest<'a> {
    pub kind: &'a str,
    pub ext: &'a str,
    pub px: u32,
}

#[derive(Deserialize)]
pub struct WireAnswer {
    pub ok: bool,
    #[serde(default)]
    pub len: usize,
    #[serde(default)]
    pub err: String,
}

/// What the helper says when it has no icon for a request.
pub const ERR_NO_ICON: &str = "no-icon";

/// Send `req` and read the answer. A helper-reported error leaves the helper usable.
pub async fn exchange<W, R>(to: &mut W, from: &mut R, req: &IconRequest) -> Result<Vec<u8>, IconError>
where
    W: AsyncWrite + Unpin,
    R: AsyncRead + AsyncBufRead + Unpin,
{
    let mut line = serde_json::to_vec(&WireRequest { kind: req.kind.as_str(), ext: &req.ext, px: req.px })
        .map_err(|_| IconError::Failed)?;
    line.push(b'\n');
    to.write_all(&line).await.map_err(|_| IconError::Failed)?;
    to.flush().await.map_err(|_| IconError::Failed)?;

    let mut answer = String::new();
    let n = from.take(MAX_LINE as u64).read_line(&mut answer).await.map_err(|_| IconError::Failed)?;
    if n == 0 || !answer.ends_with('\n') {
        return Err(IconError::Failed);
    }
    let answer: WireAnswer = serde_json::from_str(answer.trim_end()).map_err(|_| IconError::Failed)?;
    if !answer.ok {
        return Err(if answer.err == ERR_NO_ICON { IconError::NoIcon } else { IconError::Failed });
    }
    if answer.len == 0 || answer.len > MAX_PNG_BYTES {
        return Err(IconError::Failed);
    }
    let mut png = vec![0u8; answer.len];
    from.read_exact(&mut png).await.map_err(|_| IconError::Failed)?;
    Ok(png)
}

/// `exchange` that gives up after `deadline`.
pub async fn exchange_within<W, R>(
    to: &mut W,
    from: &mut R,
    req: &IconRequest,
    deadline: Duration,
) -> Result<Vec<u8>, IconError>
where
    W: AsyncWrite + Unpin,
    R: AsyncRead + AsyncBufRead + Unpin,
{
    tokio::time::timeout(deadline, exchange(to, from, req)).await.unwrap_or(Err(IconError::Timeout))
}

struct Worker {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl Worker {
    fn spawn() -> std::io::Result<Self> {
        let exe = std::env::current_exe()?;
        let mut std_cmd = std::process::Command::new(exe);
        // It only asks the shell about file types: none of this instance's identity goes with it.
        crate::backend::pane_env::sanitize_external_std_command(&mut std_cmd);
        std_cmd.arg(super::CHILD_ARG);
        let mut cmd = tokio::process::Command::from(std_cmd);
        {
            use agentmux_common::win32::NoWindow;
            cmd.no_window();
        }
        cmd.stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true);
        let mut child = cmd.spawn()?;
        let stdin = child.stdin.take().ok_or_else(|| std::io::Error::other("no stdin"))?;
        let stdout = child.stdout.take().ok_or_else(|| std::io::Error::other("no stdout"))?;
        Ok(Worker { child, stdin, stdout: BufReader::new(stdout) })
    }
}

/// The helpers and the line of requests waiting for one.
pub struct Pool {
    slots: Vec<tokio::sync::Mutex<Option<Worker>>>,
    waiting: tokio::sync::Semaphore,
    next: AtomicUsize,
}

impl Pool {
    pub fn new() -> Self {
        Pool {
            slots: (0..WORKERS).map(|_| tokio::sync::Mutex::new(None)).collect(),
            waiting: tokio::sync::Semaphore::new(QUEUE_MAX),
            next: AtomicUsize::new(0),
        }
    }

    /// Ask a helper for `req`'s picture. A helper that doesn't answer in [`DEADLINE`]
    /// is killed, the next request starts a new one, and this one is a timeout.
    pub async fn render(&self, req: &IconRequest) -> Result<Vec<u8>, IconError> {
        let _place = self.waiting.try_acquire().map_err(|_| IconError::Busy)?;
        let mut slot = None;
        for s in &self.slots {
            if let Ok(g) = s.try_lock() {
                slot = Some(g);
                break;
            }
        }
        let mut slot = match slot {
            Some(g) => g,
            None => self.slots[self.next.fetch_add(1, Ordering::Relaxed) % self.slots.len()].lock().await,
        };
        if slot.is_none() {
            *slot = Some(Worker::spawn().map_err(|e| {
                tracing::warn!(error = %e, "file icons: could not start the helper");
                IconError::Failed
            })?);
        }
        let worker = slot.as_mut().expect("just started");
        match exchange_within(&mut worker.stdin, &mut worker.stdout, req, DEADLINE).await {
            Ok(png) => Ok(png),
            Err(e @ IconError::NoIcon) => Err(e),
            Err(e) => {
                // Timed out or broke mid-answer: its pipes may hold half an answer, so it goes.
                if let Some(mut dead) = slot.take() {
                    let _ = dead.child.start_kill();
                    tokio::spawn(async move {
                        let _ = dead.child.wait().await;
                    });
                }
                if e == IconError::Timeout {
                    tracing::warn!(kind = req.kind.as_str(), ext = %req.ext, "file icons: the helper did not answer in time and was stopped");
                }
                Err(e)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::request::Kind;
    use super::*;
    use tokio::io::{duplex, AsyncBufReadExt, AsyncWriteExt};

    fn req() -> IconRequest {
        IconRequest { kind: Kind::File, ext: "rs".into(), px: 16 }
    }

    /// Run `peer` as the helper's end of a pair of pipes and `exchange_within` as srv's.
    async fn run<F, Fut>(deadline: Duration, peer: F) -> Result<Vec<u8>, IconError>
    where
        F: FnOnce(String, tokio::io::DuplexStream) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        let (mut srv_w, helper_r) = duplex(4096);
        let (helper_w, srv_r) = duplex(4096);
        // The helper reads the request first, as the real one does; `peer` gets the line and its output pipe.
        tokio::spawn(async move {
            let mut helper_r = BufReader::new(helper_r);
            let mut line = String::new();
            helper_r.read_line(&mut line).await.unwrap();
            peer(line, helper_w).await;
            drop(helper_r);
        });
        let mut srv_r = BufReader::new(srv_r);
        exchange_within(&mut srv_w, &mut srv_r, &req(), deadline).await
    }

    #[tokio::test]
    async fn a_request_is_one_json_line_and_the_answer_is_a_line_then_the_png() {
        let got = run(DEADLINE, |line, mut w| async move {
            assert_eq!(line, "{\"kind\":\"file\",\"ext\":\"rs\",\"px\":16}\n");
            w.write_all(b"{\"ok\":true,\"len\":4}\n\x89PNG").await.unwrap();
        })
        .await;
        assert_eq!(got, Ok(b"\x89PNG".to_vec()));
    }

    #[tokio::test]
    async fn a_helper_that_has_no_icon_says_so_and_a_broken_one_is_a_failure() {
        let none = run(DEADLINE, |_line, mut w| async move {
            w.write_all(b"{\"ok\":false,\"err\":\"no-icon\"}\n").await.unwrap();
        })
        .await;
        assert_eq!(none, Err(IconError::NoIcon));
        let other = run(DEADLINE, |_line, mut w| async move {
            w.write_all(b"{\"ok\":false,\"err\":\"com\"}\n").await.unwrap();
        })
        .await;
        assert_eq!(other, Err(IconError::Failed));
        let garbage = run(DEADLINE, |_line, mut w| async move { w.write_all(b"not json\n").await.unwrap() }).await;
        assert_eq!(garbage, Err(IconError::Failed));
        let closed = run(DEADLINE, |_line, w| async move { drop(w) }).await;
        assert_eq!(closed, Err(IconError::Failed), "a helper that exited");
    }

    #[tokio::test]
    async fn a_picture_that_is_empty_oversized_or_cut_short_is_refused() {
        let zero = run(DEADLINE, |_line, mut w| async move { w.write_all(b"{\"ok\":true,\"len\":0}\n").await.unwrap() }).await;
        assert_eq!(zero, Err(IconError::Failed));
        let huge = run(DEADLINE, |_line, mut w| async move {
            w.write_all(format!("{{\"ok\":true,\"len\":{}}}\n", MAX_PNG_BYTES + 1).as_bytes()).await.unwrap();
        })
        .await;
        assert_eq!(huge, Err(IconError::Failed), "before reading a byte of it");
        let short = run(DEADLINE, |_line, mut w| async move {
            w.write_all(b"{\"ok\":true,\"len\":10}\nabc").await.unwrap();
            drop(w);
        })
        .await;
        assert_eq!(short, Err(IconError::Failed));
        let long_line = run(DEADLINE, |_line, mut w| async move { w.write_all(&vec![b'x'; 2000]).await.unwrap() }).await;
        assert_eq!(long_line, Err(IconError::Failed), "an answer line has a limit");
    }

    #[tokio::test]
    async fn a_helper_that_never_answers_is_a_timeout_at_the_deadline() {
        let started = std::time::Instant::now();
        let got = run(Duration::from_millis(150), |_line, w| async move {
            // Holds the pipes open and says nothing, like a hung icon handler.
            let _keep = w;
            tokio::time::sleep(Duration::from_secs(5)).await;
        })
        .await;
        assert_eq!(got, Err(IconError::Timeout));
        assert!(started.elapsed() < Duration::from_secs(2));
    }
}
