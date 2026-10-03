// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Durable SSH terminals (P4 of
//! SPEC_REMOTE_TERMINALS_AND_DURABLE_SESSIONS_2026_10_02.md, §7).
//!
//! A pane on an SSH connection with `term:durable` set is not a local PTY. It
//! runs `ssh -T host -- agentmux-remote attach --session ID --offset N ...`
//! and speaks the helper's frames (`agentmux_remote::frame`) over that
//! channel: the shell itself lives in the helper's daemon on the host, so a
//! dropped link, a sleeping laptop or an AgentMux restart only drops this
//! `ssh`, and the pane reattaches and gets exactly what it missed.
//!
//! - Output lands in the pane's `term` file through the same append as a
//!   PTY's (`shell::handle_append_block_file`), and the next remote offset is
//!   kept beside it in that file's metadata (`remote:offset`), written right
//!   after each append, so a reattach asks for exactly what is missing.
//! - The session id is the block's `remote:session_id`, made on first start.
//! - The status stays `running` while reconnecting; only the session's own
//!   end (`Exited`) makes it `done`, which is what offers "press to restart".
//! - Reconnect never gives up while the pane is open (§7.4): backoff 1, 2, 5,
//!   10, then every 30 s. A ping every 15 s, and an `ssh` silent for 35 s is
//!   killed and replaced rather than left to its own timeout (§7.5).
//! - Closing the pane ends the session; quitting AgentMux detaches it (it
//!   keeps running on the host for the next start to reattach); a forced
//!   restart or a connection change detaches too.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use agentmux_remote::frame::{Decoder, Frame};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{mpsc, watch};

use super::{
    BlockControllerRuntimeStatus, BlockInputUnion, Controller, ShutdownFuture, StopOutcome,
    STATUS_DONE, STATUS_INIT, STATUS_RUNNING,
};
use crate::backend::eventbus::EventBus;
use crate::backend::mps;
use crate::backend::obj::{self, MetaMapType};
use crate::backend::remote::conn::SshDest;
use crate::backend::storage::filestore::FileStore;
use crate::backend::storage::store::Store;

/// Block meta: turns a pane on an SSH connection into a durable one.
pub const META_KEY_DURABLE: &str = "term:durable";
/// Block meta: the helper session this pane attaches to.
pub const META_KEY_SESSION_ID: &str = "remote:session_id";
/// `term` file meta: the next offset of the remote session's output.
pub const FILE_META_REMOTE_OFFSET: &str = "remote:offset";

/// Set when AgentMux is quitting: panes then detach (their sessions live on
/// for the next start) instead of ending their sessions as a pane close does.
static APP_EXITING: AtomicBool = AtomicBool::new(false);

/// AgentMux is quitting (`sagas::agent_teardown::app_exit`).
pub fn note_app_exiting() {
    APP_EXITING.store(true, Ordering::SeqCst);
}

/// Whether a block is a durable SSH pane.
pub fn wants(meta: &MetaMapType) -> bool {
    meta.get(META_KEY_DURABLE).and_then(|v| v.as_bool()) == Some(true)
        && matches!(
            crate::backend::remote::ConnTarget::parse(&obj::meta_get_string(
                meta,
                super::META_KEY_CONNECTION,
                ""
            )),
            Ok(crate::backend::remote::ConnTarget::Ssh(_))
        )
}

/// The pane's size from its runtime options, as `(cols, rows)`; 80 x 24 when
/// it has none yet (the frontend resizes right after it attaches).
fn term_size(rt_opts: &Option<serde_json::Value>) -> (u16, u16) {
    rt_opts
        .as_ref()
        .and_then(|v| serde_json::from_value::<obj::RuntimeOpts>(v.clone()).ok())
        .filter(|rt| rt.termsize.cols > 0 && rt.termsize.rows > 0)
        .map(|rt| {
            (
                rt.termsize.cols.clamp(1, 1000) as u16,
                rt.termsize.rows.clamp(1, 1000) as u16,
            )
        })
        .unwrap_or((80, 24))
}

/// How long to wait before reconnect attempt `attempt` (0-based): 1, 2, 5, 10,
/// then 30 s for ever. Never a last attempt: a cap is how a recoverable pane
/// becomes a dead one.
pub fn backoff(attempt: u32) -> Duration {
    Duration::from_secs(match attempt {
        0 => 1,
        1 => 2,
        2 => 5,
        3 => 10,
        _ => 30,
    })
}

/// What part of an `Output` frame the pane has not seen yet, given the next
/// offset it expects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Accept {
    /// All of it was already appended (a replay overlapping what was seen).
    Seen,
    /// Append `data[skip..]`; the next expected offset becomes `next`.
    Append { skip: usize, next: u64 },
    /// Bytes `from..to` never arrived; then append all of `data`.
    Gap { from: u64, to: u64, next: u64 },
}

/// Decide what to do with output at `offset`, when the pane expects `expected`.
pub fn accept(expected: u64, offset: u64, len: usize) -> Accept {
    let end = offset + len as u64;
    if end <= expected {
        Accept::Seen
    } else if offset <= expected {
        Accept::Append {
            skip: (expected - offset) as usize,
            next: end,
        }
    } else {
        Accept::Gap {
            from: expected,
            to: offset,
            next: end,
        }
    }
}

/// Where the helper is installed on the host: a versioned path, so two
/// AgentMux versions can use one host (spec §6.2). `~` is the remote home.
pub fn helper_path() -> String {
    format!(
        "~/.agentmux-remote/bin/{}/agentmux-remote",
        env!("CARGO_PKG_VERSION")
    )
}

/// The command `ssh` runs on the host.
pub fn attach_command(session: &str, offset: u64, cols: u16, rows: u16) -> String {
    format!(
        "{} attach --session {session} --offset {offset} --cols {cols} --rows {rows}",
        helper_path()
    )
}

/// `ssh` exits 127 when the remote shell cannot find the command: no helper.
const EXIT_COMMAND_NOT_FOUND: i32 = 127;
const PING_EVERY: Duration = Duration::from_secs(15);
const STALLED_AFTER: Duration = Duration::from_secs(35);

/// How the pane leaves its session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Leave {
    Stay,
    /// Drop the connection; the session keeps running on the host.
    Detach,
    /// End the session: its shell and everything it started.
    End,
}

struct Inner {
    status: String,
    version: i32,
    exit_code: i32,
    conn_name: String,
    spawn_ts_ms: Option<i64>,
    input_tx: Option<mpsc::UnboundedSender<Frame>>,
    leave_tx: Option<watch::Sender<Leave>>,
    done_rx: Option<watch::Receiver<bool>>,
}

pub struct DurableSshController {
    block_id: String,
    broker: Option<Arc<mps::Broker>>,
    event_bus: Option<Arc<EventBus>>,
    mstore: Option<Arc<Store>>,
    filestore: Option<Arc<FileStore>>,
    inner: Mutex<Inner>,
}

impl DurableSshController {
    pub fn new(
        block_id: String,
        broker: Option<Arc<mps::Broker>>,
        event_bus: Option<Arc<EventBus>>,
        mstore: Option<Arc<Store>>,
        filestore: Option<Arc<FileStore>>,
    ) -> Self {
        Self {
            block_id,
            broker,
            event_bus,
            mstore,
            filestore,
            inner: Mutex::new(Inner {
                status: STATUS_INIT.to_string(),
                version: 0,
                exit_code: 0,
                conn_name: String::new(),
                spawn_ts_ms: None,
                input_tx: None,
                leave_tx: None,
                done_rx: None,
            }),
        }
    }

    fn snapshot(&self) -> BlockControllerRuntimeStatus {
        let inner = self.inner.lock().unwrap();
        BlockControllerRuntimeStatus {
            blockid: self.block_id.clone(),
            version: inner.version,
            shellprocstatus: inner.status.clone(),
            shellprocconnname: inner.conn_name.clone(),
            shellprocexitcode: inner.exit_code,
            spawn_ts_ms: inner.spawn_ts_ms,
            ..Default::default()
        }
    }

    fn publish(&self) {
        if let Some(b) = &self.broker {
            super::publish_controller_status(b, &self.snapshot());
        }
    }

    /// The pane's session id, made and recorded on first start.
    fn session_id(&self, meta: &MetaMapType) -> String {
        let existing = obj::meta_get_string(meta, META_KEY_SESSION_ID, "");
        if !existing.is_empty() {
            return existing;
        }
        let id = format!("amx-{}", uuid::Uuid::new_v4().simple());
        self.write_block_meta(META_KEY_SESSION_ID, serde_json::json!(id));
        id
    }

    fn write_block_meta(&self, key: &str, value: serde_json::Value) {
        let Some(store) = &self.mstore else { return };
        let oref = format!("block:{}", self.block_id);
        let mut m = MetaMapType::new();
        m.insert(key.to_string(), value);
        if crate::server::service::update_object_meta(store, &oref, &m).is_err() {
            return;
        }
        if let (Some(bus), Ok(block)) = (
            &self.event_bus,
            store.must_get::<obj::Block>(&self.block_id),
        ) {
            bus.broadcast_event(&crate::backend::eventbus::WSEventType {
                eventtype: "waveobj:update".to_string(),
                oref: oref.clone(),
                data: serde_json::to_value(&obj::MuxObjUpdate {
                    updatetype: "update".into(),
                    otype: "block".into(),
                    oid: self.block_id.clone(),
                    obj: Some(obj::mux_obj_to_value(&block)),
                })
                .ok(),
            });
        }
    }

    fn leave(&self, how: Leave) {
        let inner = self.inner.lock().unwrap();
        if let Some(tx) = &inner.leave_tx {
            let _ = tx.send(how);
        }
    }
}

impl Controller for DurableSshController {
    fn start(
        &self,
        block_meta: MetaMapType,
        rt_opts: Option<serde_json::Value>,
        _force: bool,
    ) -> Result<(), String> {
        let conn = obj::meta_get_string(&block_meta, super::META_KEY_CONNECTION, "");
        let dest = match crate::backend::remote::ConnTarget::parse(&conn) {
            Ok(crate::backend::remote::ConnTarget::Ssh(d)) => d,
            _ => return Err(format!("{conn:?} is not an SSH connection")),
        };
        let ssh_path = crate::backend::remote::ssh::binary()
            .ok_or_else(crate::backend::remote::ssh::missing_binary_message)?;
        {
            let inner = self.inner.lock().unwrap();
            if inner.status == STATUS_RUNNING {
                return Ok(());
            }
        }
        let session = self.session_id(&block_meta);
        let size = term_size(&rt_opts);
        let (input_tx, input_rx) = mpsc::unbounded_channel();
        let (leave_tx, leave_rx) = watch::channel(Leave::Stay);
        let (done_tx, done_rx) = watch::channel(false);
        {
            let mut inner = self.inner.lock().unwrap();
            inner.status = STATUS_RUNNING.to_string();
            inner.version += 1;
            inner.exit_code = 0;
            inner.conn_name = conn.clone();
            inner.spawn_ts_ms = Some(agentmux_common::time::now_ms());
            inner.input_tx = Some(input_tx);
            inner.leave_tx = Some(leave_tx);
            inner.done_rx = Some(done_rx);
        }
        self.publish();
        let run = Run {
            block_id: self.block_id.clone(),
            conn,
            dest,
            ssh_path,
            control_dir: crate::backend::remote::ssh::control_dir(
                &crate::backend::base::get_mux_config_dir(),
            ),
            session,
            size,
            broker: self.broker.clone(),
            filestore: self.filestore.clone(),
        };
        let this = super::get_controller(&self.block_id);
        tokio::spawn(async move {
            let ended = run.run(input_rx, leave_rx).await;
            let _ = done_tx.send(true);
            // Only the session's own end (or a helper that is not there) makes
            // the pane `done`; a detach or an end asked for leaves that to
            // whoever asked.
            if let Some(code) = ended {
                if let Some(ctrl) = this
                    .as_ref()
                    .and_then(|c| c.as_any().downcast_ref::<DurableSshController>())
                {
                    {
                        let mut inner = ctrl.inner.lock().unwrap();
                        inner.status = STATUS_DONE.to_string();
                        inner.version += 1;
                        inner.exit_code = code;
                        inner.input_tx = None;
                    }
                    // The session is over: the next start makes a new one.
                    ctrl.write_block_meta(META_KEY_SESSION_ID, serde_json::Value::Null);
                    ctrl.publish();
                }
            }
        });
        Ok(())
    }

    fn stop(&self, _graceful: bool, new_status: &str) -> Result<(), String> {
        let how = if APP_EXITING.load(Ordering::SeqCst) {
            Leave::Detach
        } else {
            Leave::End
        };
        self.leave(how);
        let mut inner = self.inner.lock().unwrap();
        inner.status = new_status.to_string();
        inner.version += 1;
        inner.input_tx = None;
        Ok(())
    }

    /// A forced restart or a connection change: detach, so the session is
    /// still there if this pane comes back to it.
    fn stop_for_replace(&self, new_status: &str) -> Result<(), String> {
        self.leave(Leave::Detach);
        let mut inner = self.inner.lock().unwrap();
        inner.status = new_status.to_string();
        inner.version += 1;
        inner.input_tx = None;
        Ok(())
    }

    /// Pane close ends the session; AgentMux quitting detaches it. Waits for
    /// the `ssh` to deliver that before the deadline.
    fn shutdown(&self, deadline: std::time::Instant) -> ShutdownFuture {
        let how = if APP_EXITING.load(Ordering::SeqCst) {
            Leave::Detach
        } else {
            Leave::End
        };
        self.leave(how);
        let done_rx = self.inner.lock().unwrap().done_rx.clone();
        {
            let mut inner = self.inner.lock().unwrap();
            inner.status = STATUS_DONE.to_string();
            inner.version += 1;
            inner.input_tx = None;
        }
        Box::pin(async move {
            let Some(mut rx) = done_rx else {
                return StopOutcome::NotRunning;
            };
            let wait = deadline.saturating_duration_since(std::time::Instant::now());
            let _ = tokio::time::timeout(wait, async {
                while !*rx.borrow() {
                    if rx.changed().await.is_err() {
                        break;
                    }
                }
            })
            .await;
            StopOutcome::Stopped
        })
    }

    fn get_runtime_status(&self) -> BlockControllerRuntimeStatus {
        self.snapshot()
    }

    fn send_input(&self, input: BlockInputUnion, _seq: Option<u64>) -> Result<(), String> {
        let inner = self.inner.lock().unwrap();
        let tx = inner.input_tx.as_ref().ok_or("controller is not running")?;
        if let Some(data) = input.input_data {
            let _ = tx.send(Frame::Input(data));
        }
        if let Some(ts) = input.term_size {
            let _ = tx.send(Frame::Resize {
                cols: ts.cols.clamp(1, 1000) as u16,
                rows: ts.rows.clamp(1, 1000) as u16,
            });
        }
        if input.sig_name.is_some() {
            let _ = tx.send(Frame::End);
        }
        Ok(())
    }

    fn controller_type(&self) -> &str {
        super::BLOCK_CONTROLLER_SHELL
    }

    fn block_id(&self) -> &str {
        &self.block_id
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// One durable pane's connection loop.
struct Run {
    block_id: String,
    conn: String,
    dest: SshDest,
    ssh_path: std::path::PathBuf,
    /// ssh's connection-sharing directory (macOS and Linux), if any.
    control_dir: Option<std::path::PathBuf>,
    session: String,
    size: (u16, u16),
    broker: Option<Arc<mps::Broker>>,
    filestore: Option<Arc<FileStore>>,
}

/// How one `ssh` ended.
enum Ended {
    /// The session's shell exited with this code.
    Exited(i32),
    /// The pane left (detach or end).
    Left,
    /// The link dropped or stalled: `ssh`'s exit code, and whether it reached
    /// the session at all.
    Dropped {
        code: Option<i32>,
        attached: bool,
        stderr: String,
    },
}

impl Run {
    /// Connect, and reconnect for as long as the pane wants the session.
    /// `Some(code)` when the session (or the attempt to reach it) is over and
    /// the pane should show it done; `None` when the pane left.
    async fn run(
        mut self,
        mut input_rx: mpsc::UnboundedReceiver<Frame>,
        mut leave_rx: watch::Receiver<Leave>,
    ) -> Option<i32> {
        let mut attempt = 0u32;
        let mut noted_drop = false;
        loop {
            if *leave_rx.borrow() != Leave::Stay {
                return None;
            }
            // Keystrokes typed while disconnected are not replayed into a
            // shell the user cannot see; the last size is kept.
            while let Ok(f) = input_rx.try_recv() {
                if let Frame::Resize { cols, rows } = f {
                    self.size = (cols, rows);
                }
            }
            match self.attach_once(&mut input_rx, &mut leave_rx).await {
                Ended::Exited(code) => return Some(code),
                Ended::Left => return None,
                Ended::Dropped {
                    code,
                    attached,
                    stderr,
                } => {
                    if !attached && code == Some(EXIT_COMMAND_NOT_FOUND) {
                        self.note(&format!(
                            "AgentMux's helper is not installed on {} ({}), so this pane cannot be durable. \
                             Turn off term:durable for a plain SSH terminal.",
                            self.conn,
                            helper_path()
                        ))
                        .await;
                        return Some(EXIT_COMMAND_NOT_FOUND);
                    }
                    if attached {
                        attempt = 0;
                        noted_drop = false;
                    }
                    if !noted_drop {
                        let why = stderr.lines().last().unwrap_or("").trim().to_string();
                        let why = if why.is_empty() {
                            String::new()
                        } else {
                            format!(" ({why})")
                        };
                        self.note(&format!(
                            "Disconnected from {}{why}; reconnecting…",
                            self.conn
                        ))
                        .await;
                        noted_drop = true;
                    }
                    let wait = backoff(attempt);
                    attempt = attempt.saturating_add(1);
                    tokio::select! {
                        _ = tokio::time::sleep(wait) => {}
                        _ = leave_rx.changed() => return None,
                    }
                }
            }
        }
    }

    async fn attach_once(
        &mut self,
        input_rx: &mut mpsc::UnboundedReceiver<Frame>,
        leave_rx: &mut watch::Receiver<Leave>,
    ) -> Ended {
        use crate::backend::remote::{ssh, status};
        let mut expected = self.read_offset();
        let remote = attach_command(&self.session, expected, self.size.0, self.size.1);
        // No terminal on the remote side: frames, not a session, go over it.
        let mut args = ssh::launch(&self.dest, &remote, &[], "", self.control_dir.as_deref());
        args[0] = "-T".to_string();
        let mut cmd = tokio::process::Command::new(&self.ssh_path);
        cmd.args(&args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        #[cfg(windows)]
        {
            use agentmux_common::win32::NoWindow;
            cmd.no_window();
        }
        crate::backend::pane_env::sanitize_process_command(&mut cmd);
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                return Ended::Dropped {
                    code: None,
                    attached: false,
                    stderr: format!("could not run ssh: {e}"),
                };
            }
        };
        if let Some(pid) = child.id() {
            crate::backend::process_tracker::registry::track_spawned(&self.block_id, pid);
        }
        let (mut stdin, mut stdout, mut stderr) = (
            child.stdin.take().unwrap(),
            child.stdout.take().unwrap(),
            child.stderr.take().unwrap(),
        );
        let stderr_task = tokio::spawn(async move {
            let mut buf = Vec::new();
            let _ = stderr.read_to_end(&mut buf).await;
            let text = String::from_utf8_lossy(&buf).into_owned();
            text.chars()
                .rev()
                .take(2000)
                .collect::<String>()
                .chars()
                .rev()
                .collect::<String>()
        });

        let mut decoder = Decoder::new();
        let mut buf = vec![0u8; 64 * 1024];
        let mut attached = false;
        let mut last_heard = tokio::time::Instant::now();
        let mut ping = tokio::time::interval(PING_EVERY);
        ping.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let ended = loop {
            tokio::select! {
                read = stdout.read(&mut buf) => {
                    let n = match read { Ok(0) | Err(_) => break None, Ok(n) => n };
                    last_heard = tokio::time::Instant::now();
                    let Ok(frames) = decoder.push(&buf[..n]) else { break None };
                    let mut exited: Option<i32> = None;
                    for frame in frames {
                        match frame {
                            Frame::Hello { created, .. } => {
                                attached = true;
                                status::pane_started(self.broker.as_deref(), &self.conn);
                                if created && expected > 0 {
                                    self.note("The previous session on this host is gone (the host restarted or it was ended); this is a new one.").await;
                                    expected = 0;
                                    self.write_offset(0);
                                }
                            }
                            Frame::Output { offset, data } => match accept(expected, offset, data.len()) {
                                Accept::Seen => {}
                                Accept::Append { skip, next } => {
                                    self.append(data[skip..].to_vec()).await;
                                    expected = next;
                                    self.write_offset(next);
                                }
                                Accept::Gap { from, to, next } => {
                                    self.note(&format!("{} bytes of output were lost while detached.", to - from)).await;
                                    self.append(data).await;
                                    expected = next;
                                    self.write_offset(next);
                                }
                            },
                            Frame::Lost { from, to } => {
                                self.note(&format!("{} bytes of output were lost while detached.", to.saturating_sub(from))).await;
                                expected = expected.max(to);
                                self.write_offset(expected);
                            }
                            Frame::Exited { code } => exited = Some(code),
                            Frame::Error(msg) => self.note(&format!("The session on {}: {msg}", self.conn)).await,
                            _ => {}
                        }
                        if let Some(code) = exited {
                            if attached {
                                status::pane_ended(self.broker.as_deref(), &self.conn, None);
                            }
                            let _ = child.kill().await;
                            return Ended::Exited(code);
                        }
                    }
                }
                frame = input_rx.recv() => {
                    let Some(frame) = frame else { break Some(Leave::Detach) };
                    if let Frame::Resize { cols, rows } = frame {
                        self.size = (cols, rows);
                    }
                    let end = frame == Frame::End;
                    if stdin.write_all(&frame.encode()).await.is_err() {
                        break None;
                    }
                    if end {
                        break Some(Leave::End);
                    }
                }
                _ = ping.tick() => {
                    if last_heard.elapsed() > STALLED_AFTER {
                        break None; // stalled: replace this ssh rather than wait on it
                    }
                    if stdin.write_all(&Frame::Ping.encode()).await.is_err() {
                        break None;
                    }
                }
                changed = leave_rx.changed() => {
                    let how = if changed.is_ok() { *leave_rx.borrow() } else { Leave::Detach };
                    if how != Leave::Stay {
                        break Some(how);
                    }
                }
            }
        };
        if attached {
            status::pane_ended(self.broker.as_deref(), &self.conn, None);
        }
        if let Some(how) = ended {
            // The pane left: say how, give ssh a moment to carry it, then go.
            let frame = if how == Leave::End {
                Frame::End
            } else {
                Frame::Detach
            };
            let _ = stdin.write_all(&frame.encode()).await;
            let _ = stdin.shutdown().await;
            let _ = tokio::time::timeout(Duration::from_secs(2), child.wait()).await;
            let _ = child.kill().await;
            return Ended::Left;
        }
        let _ = child.kill().await;
        let code = child.wait().await.ok().and_then(|s| s.code());
        let stderr = stderr_task.await.unwrap_or_default();
        Ended::Dropped {
            code,
            attached,
            stderr,
        }
    }

    fn read_offset(&self) -> u64 {
        self.filestore
            .as_ref()
            .and_then(|fs| fs.stat(&self.block_id, "term").ok().flatten())
            .and_then(|f| f.meta.get(FILE_META_REMOTE_OFFSET).and_then(|v| v.as_u64()))
            .unwrap_or(0)
    }

    fn write_offset(&self, offset: u64) {
        let Some(fs) = &self.filestore else { return };
        let mut meta = crate::backend::storage::filestore::FileMeta::new();
        meta.insert(
            FILE_META_REMOTE_OFFSET.to_string(),
            serde_json::json!(offset),
        );
        let _ = fs.write_meta(&self.block_id, "term", meta, true);
    }

    /// Append to the pane's terminal, exactly as a PTY's output is appended.
    async fn append(&self, data: Vec<u8>) {
        let Some(broker) = self.broker.clone() else {
            return;
        };
        let (block_id, filestore) = (self.block_id.clone(), self.filestore.clone());
        let _ = tokio::task::spawn_blocking(move || {
            super::shell::handle_append_block_file(
                &broker,
                &block_id,
                "term",
                &data,
                filestore.as_ref(),
                None,
            );
        })
        .await;
    }

    /// A line from AgentMux in the pane, dimmed, on its own line.
    async fn note(&self, text: &str) {
        self.append(format!("\r\n\x1b[2m[{text}]\x1b[0m\r\n").into_bytes())
            .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconnecting_backs_off_and_never_stops() {
        let waits: Vec<u64> = (0..8).map(|a| backoff(a).as_secs()).collect();
        assert_eq!(waits, [1, 2, 5, 10, 30, 30, 30, 30]);
        assert_eq!(backoff(u32::MAX).as_secs(), 30);
    }

    #[test]
    fn output_is_appended_once_and_gaps_are_named() {
        assert_eq!(accept(10, 0, 5), Accept::Seen, "all before what was seen");
        assert_eq!(accept(10, 5, 5), Accept::Seen);
        assert_eq!(
            accept(10, 5, 8),
            Accept::Append { skip: 5, next: 13 },
            "overlap: only the new part"
        );
        assert_eq!(accept(10, 10, 4), Accept::Append { skip: 0, next: 14 });
        assert_eq!(
            accept(10, 12, 3),
            Accept::Gap {
                from: 10,
                to: 12,
                next: 15
            }
        );
    }

    #[test]
    fn the_attach_command_names_the_versioned_helper() {
        let c = attach_command("amx-1", 42, 120, 40);
        assert!(c.starts_with("~/.agentmux-remote/bin/"), "{c}");
        assert!(
            c.ends_with("/agentmux-remote attach --session amx-1 --offset 42 --cols 120 --rows 40"),
            "{c}"
        );
    }

    /// A stand-in for `ssh` + the helper, speaking the helper's real frames:
    /// the first connection attaches, sends "hello" and drops (exit 255, a
    /// network drop); the reconnect must ask for exactly offset 7, gets
    /// "world" and the session's end.
    const FAKE_SSH: &str = r#"
import os, struct, sys, time
remote = sys.argv[-1].split()
offset = int(remote[remote.index('--offset') + 1])
state = os.environ['FAKE_SSH_STATE']
n = int(open(state).read()) if os.path.exists(state) else 0
open(state, 'w').write(str(n + 1))
with open(os.environ['FAKE_SSH_LOG'], 'ab') as log:
    log.write(b'run %d offset %d\n' % (n, offset))
out = sys.stdout.buffer
def frame(kind, payload):
    out.write(struct.pack('>BI', kind, len(payload)) + payload)
    out.flush()
stream = b'hello\r\n' + b'world\r\n'
if n == 0:
    frame(14, struct.pack('>Q', 0) + bytes([1]) + b'amx-test')
    frame(10, struct.pack('>Q', 0) + stream[:7])
    sys.exit(255)
frame(14, struct.pack('>Q', 7) + bytes([0]) + b'amx-test')
frame(10, struct.pack('>Q', offset) + stream[offset:])
frame(12, struct.pack('>i', 0))
time.sleep(5)
"#;

    fn python() -> Option<std::path::PathBuf> {
        let names: &[&str] = if cfg!(windows) {
            &["python", "py"]
        } else {
            &["python3", "python"]
        };
        names.iter().find_map(|n| which::which(n).ok())
    }

    /// The reconnect loop for real: a dropped link reattaches at exactly the
    /// offset it had reached, nothing is appended twice, and the session's
    /// own end ends the pane's run.
    #[tokio::test]
    async fn a_dropped_link_reattaches_at_its_offset_and_appends_nothing_twice() {
        let Some(python) = python() else {
            eprintln!("skipped: no python to stand in for ssh");
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("fake_ssh.py");
        std::fs::write(&script, FAKE_SSH).unwrap();
        let ssh_path = if cfg!(windows) {
            let cmd = dir.path().join("fake_ssh.cmd");
            std::fs::write(
                &cmd,
                format!("@\"{}\" \"{}\" %*\r\n", python.display(), script.display()),
            )
            .unwrap();
            cmd
        } else {
            let sh = dir.path().join("fake_ssh");
            std::fs::write(
                &sh,
                format!(
                    "#!/bin/sh\nexec \"{}\" \"{}\" \"$@\"\n",
                    python.display(),
                    script.display()
                ),
            )
            .unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&sh, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
            sh
        };
        std::env::set_var("FAKE_SSH_STATE", dir.path().join("state"));
        std::env::set_var("FAKE_SSH_LOG", dir.path().join("log"));

        let state = crate::server::tests::test_state();
        let block = "durable-test-block";
        let run = Run {
            block_id: block.to_string(),
            conn: "fakehost".to_string(),
            dest: SshDest {
                destination: "fakehost".to_string(),
                port: None,
            },
            ssh_path,
            control_dir: None,
            session: "amx-test".to_string(),
            size: (80, 24),
            broker: Some(state.broker.clone()),
            filestore: Some(state.filestore.clone()),
        };
        let (_input_tx, input_rx) = mpsc::unbounded_channel();
        let (_leave_tx, leave_rx) = watch::channel(Leave::Stay);
        let ended = tokio::time::timeout(Duration::from_secs(30), run.run(input_rx, leave_rx))
            .await
            .expect("the run ends");
        assert_eq!(ended, Some(0), "the session's own end ends the run");

        let log = std::fs::read_to_string(dir.path().join("log")).unwrap();
        assert_eq!(
            log, "run 0 offset 0\nrun 1 offset 7\n",
            "the reconnect asks for exactly what it missed"
        );
        let term = state.filestore.read_file(block, "term").unwrap().unwrap();
        let text = String::from_utf8_lossy(&term).into_owned();
        assert_eq!(text.matches("hello").count(), 1, "{text:?}");
        assert_eq!(text.matches("world").count(), 1, "{text:?}");
        assert!(
            text.find("hello") < text.find("reconnecting")
                && text.find("reconnecting") < text.find("world"),
            "{text:?}"
        );
        let offset = state
            .filestore
            .stat(block, "term")
            .unwrap()
            .unwrap()
            .meta
            .get(FILE_META_REMOTE_OFFSET)
            .cloned();
        assert_eq!(offset, Some(serde_json::json!(14)));
    }

    #[test]
    fn only_an_ssh_pane_with_durable_on_is_durable() {
        let meta = |conn: &str, durable: Option<bool>| {
            let mut m = MetaMapType::new();
            m.insert(
                super::super::META_KEY_CONNECTION.into(),
                serde_json::json!(conn),
            );
            if let Some(d) = durable {
                m.insert(META_KEY_DURABLE.into(), serde_json::json!(d));
            }
            m
        };
        assert!(wants(&meta("area54", Some(true))));
        assert!(!wants(&meta("area54", None)));
        assert!(!wants(&meta("area54", Some(false))));
        assert!(!wants(&meta("local", Some(true))));
        assert!(!wants(&meta("wsl://Ubuntu", Some(true))));
    }
}
