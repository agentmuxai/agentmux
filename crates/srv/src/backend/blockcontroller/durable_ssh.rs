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
//! - `ssh` has no terminal here, so its prompts (a password, a passphrase, a
//!   new host key) go to the user through the askpass bridge
//!   (`remote::askpass`). A login it still cannot make stops the pane with
//!   the reason rather than asking again every 30 s.

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
use crate::backend::remote::host::HostSsh;
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

/// Whether a shell pane runs durable (spec §7.1). Only on an SSH connection:
/// the pane's own `term:durable` first, then its connection's settings, then
/// the global setting; with none of them set, yes on a host the helper has
/// answered on (`remote::helper_hosts`).
pub fn wants(meta: &MetaMapType, config: Option<&crate::backend::wconfig::FullConfigType>) -> bool {
    let conn = obj::meta_get_string(meta, super::META_KEY_CONNECTION, "");
    if !matches!(
        crate::backend::remote::ConnTarget::parse(&conn),
        Ok(crate::backend::remote::ConnTarget::Ssh(_))
    ) {
        return false;
    }
    // An agent's SSH shell (`PtyShell`, marked by http_pty_shell.rs) is never
    // durable, whatever the defaults: its ssh's prompts must reach the user as
    // the agent's (its own askpass grant), and the agent tools do not reattach
    // a session yet (the `Shell` tool's `durable`, spec §8.1, still to come).
    if !obj::meta_get_string(
        meta,
        crate::backend::remote::askpass::META_KEY_AGENT_BLOCK,
        "",
    )
    .is_empty()
    {
        return false;
    }
    let conn_setting = config.and_then(|c| {
        c.connections
            .iter()
            .find(|(name, _)| crate::backend::remote::conn::same_connection(name, &conn))
            .and_then(|(_, k)| k.term_durable)
    });
    resolve_durable(
        meta.get(META_KEY_DURABLE).and_then(|v| v.as_bool()),
        conn_setting,
        config.and_then(|c| c.settings.term_durable),
        || crate::backend::remote::helper_hosts::known(&conn),
    )
}

/// §7.1's order: pane, connection, global, then whether the host has the
/// helper (asked only when nothing is set).
pub fn resolve_durable(
    pane: Option<bool>,
    connection: Option<bool>,
    global: Option<bool>,
    host_has_helper: impl FnOnce() -> bool,
) -> bool {
    pane.or(connection)
        .or(global)
        .unwrap_or_else(host_has_helper)
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

/// A session id as the helper accepts it: letters, digits, `-` and `_`, up to
/// 64 (`agentmux-remote`'s `daemon::valid_session_id`).
pub fn valid_session_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
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
/// `ssh`'s own failures (it could not connect or log in) exit 255.
const EXIT_SSH_FAILED: i32 = 255;

/// ssh could not log in, and trying again would only fail (or ask) again:
/// refused credentials, or a host key it will not accept.
pub fn login_refused(stderr: &str) -> bool {
    [
        "Permission denied (",
        "Too many authentication failures",
        "No more authentication methods",
        "Host key verification failed",
        "REMOTE HOST IDENTIFICATION HAS CHANGED",
    ]
    .iter()
    .any(|p| stderr.contains(p))
}

/// How a pane's run ended, when the pane should show it done.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Done {
    code: i32,
    /// The session is gone (its shell exited, or the pane ended it): the next
    /// start makes a new one. Otherwise it may still be on the host, and the
    /// pane keeps its id to reattach.
    session_over: bool,
}
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
    /// srv's auth key, for the askpass helper to reach srv with.
    auth_key: String,
    /// Settings, for whether the pane is still durable ([`wants`]).
    config: Option<Arc<crate::backend::wconfig::ConfigState>>,
    inner: Mutex<Inner>,
}

impl DurableSshController {
    pub fn new(
        block_id: String,
        broker: Option<Arc<mps::Broker>>,
        event_bus: Option<Arc<EventBus>>,
        mstore: Option<Arc<Store>>,
        filestore: Option<Arc<FileStore>>,
        auth_key: String,
        config: Option<Arc<crate::backend::wconfig::ConfigState>>,
    ) -> Self {
        Self {
            block_id,
            broker,
            event_bus,
            mstore,
            filestore,
            auth_key,
            config,
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
            durable: true,
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
        // Restored, imported or edited meta may hold anything; this id goes
        // into a command the remote shell parses, so only the helper's own
        // grammar is used, and anything else is replaced with a fresh id.
        let existing = obj::meta_get_string(meta, META_KEY_SESSION_ID, "");
        if valid_session_id(&existing) {
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

/// Show the pane `done` with `done.code`; a session that is over is
/// forgotten, so the next start makes a new one.
fn show_done(this: Option<&Arc<dyn super::Controller>>, done: Done) {
    let Some(ctrl) = this.and_then(|c| c.as_any().downcast_ref::<DurableSshController>()) else {
        return;
    };
    {
        let mut inner = ctrl.inner.lock().unwrap();
        inner.status = STATUS_DONE.to_string();
        inner.version += 1;
        inner.exit_code = done.code;
        inner.input_tx = None;
    }
    if done.session_over {
        ctrl.write_block_meta(META_KEY_SESSION_ID, serde_json::Value::Null);
    }
    ctrl.publish();
}

impl Controller for DurableSshController {
    fn start(
        &self,
        block_meta: MetaMapType,
        rt_opts: Option<serde_json::Value>,
        _force: bool,
    ) -> Result<(), String> {
        let conn = obj::meta_get_string(&block_meta, super::META_KEY_CONNECTION, "");
        let mut host = HostSsh::for_connection(&conn)?;
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
        // ssh's prompts go to the user (this pane's window), revoked when the
        // run ends.
        let askpass_grant = host.ask_user_in(&self.block_id, &conn, &self.auth_key);
        let run = Run {
            block_id: self.block_id.clone(),
            conn,
            host,
            session,
            size,
            broker: self.broker.clone(),
            filestore: self.filestore.clone(),
            _askpass_grant: askpass_grant,
        };
        let this = super::get_controller(&self.block_id);
        let shown = this.clone();
        tokio::spawn(async move {
            let ended = run
                .run(
                    input_rx,
                    leave_rx,
                    Box::new(move |done| show_done(shown.as_ref(), done)),
                )
                .await;
            let _ = done_tx.send(true);
            // Only the session's own end (or a helper that is not there) makes
            // the pane `done`; a detach or an end asked for leaves that to
            // whoever asked.
            if let Some(done) = ended {
                show_done(this.as_ref(), done);
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
        // Durable turned off (or the connection changed away from SSH): the
        // pane will not come back to this session, so end it rather than
        // leave a shell running on the host that nothing can reach.
        let still_durable = self
            .mstore
            .as_ref()
            .and_then(|s| s.get::<obj::Block>(&self.block_id).ok().flatten())
            .map(|b| {
                wants(
                    &b.meta,
                    self.config.as_ref().map(|c| c.get_full_config()).as_deref(),
                )
            })
            .unwrap_or(true);
        self.leave(if still_durable {
            Leave::Detach
        } else {
            Leave::End
        });
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
        // A signal from the pane (restart, kill) ends the remote session; the
        // run then finishes and the pane offers a restart (Ended::EndedHere).
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

/// Append to the pane's `term` file, then tell the frontend where the bytes
/// landed (the same `blockfile` append event a PTY's output produces). Unlike
/// the PTY path's `handle_append_block_file`, which logs and swallows a store
/// error, this says whether the bytes were stored, so the remote offset never
/// moves past output that was lost.
fn append_checked(broker: &mps::Broker, fs: &FileStore, block_id: &str, data: &[u8]) -> bool {
    use base64::Engine;
    match fs.stat(block_id, "term") {
        Ok(Some(_)) => {}
        Ok(None) => {
            if let Err(e) = fs.make_file(
                block_id,
                "term",
                std::collections::HashMap::new(),
                crate::backend::storage::filestore::FileOpts::default(),
            ) {
                if !matches!(e, crate::backend::storage::error::StoreError::AlreadyExists) {
                    tracing::warn!(block_id = %block_id, error = %e, "durable ssh: could not create term");
                    return false;
                }
            }
        }
        Err(e) => {
            tracing::warn!(block_id = %block_id, error = %e, "durable ssh: term stat failed");
            return false;
        }
    }
    let start = match fs.append_data_at(block_id, "term", data) {
        Ok(start) => start.max(0) as u64,
        Err(e) => {
            tracing::warn!(block_id = %block_id, error = %e, "durable ssh: term append failed");
            return false;
        }
    };
    let event_data = mps::WSFileEventData {
        zoneid: block_id.to_string(),
        filename: "term".to_string(),
        fileop: mps::FILE_OP_APPEND.to_string(),
        data64: base64::engine::general_purpose::STANDARD.encode(data),
        offset: Some(start),
        pos: Vec::new(),
        echo: None,
    };
    broker.publish(mps::MuxEvent {
        event: mps::EVENT_BLOCK_FILE.to_string(),
        scopes: vec![format!("block:{block_id}")],
        sender: String::new(),
        persist: 0,
        data: serde_json::to_value(&event_data).ok(),
    });
    true
}

/// One durable pane's connection loop.
struct Run {
    block_id: String,
    conn: String,
    /// The host, with askpass for ssh's prompts.
    host: HostSsh,
    session: String,
    size: (u16, u16),
    broker: Option<Arc<mps::Broker>>,
    filestore: Option<Arc<FileStore>>,
    /// The askpass grant behind `host`'s prompts (revoked on drop).
    _askpass_grant: Option<crate::backend::remote::askpass::Revoke>,
}

/// How one `ssh` ended.
enum Ended {
    /// The session's shell exited with this code.
    Exited(i32),
    /// The pane left (detach or end) because it was stopped or closed.
    Left,
    /// The pane's own input ended the session (a signal).
    EndedHere,
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
    /// `Some` when the session (or the attempt to reach it) is over and the
    /// pane should show it done; `None` when the pane left. A refused login
    /// shows the pane done through `stopped` and stays until the pane leaves
    /// (see there).
    async fn run(
        mut self,
        mut input_rx: mpsc::UnboundedReceiver<Frame>,
        mut leave_rx: watch::Receiver<Leave>,
        stopped: Box<dyn FnOnce(Done) + Send>,
    ) -> Option<Done> {
        let mut attempt = 0u32;
        let mut noted_drop = false;
        let mut installed_once = false;
        loop {
            let how = *leave_rx.borrow();
            if how != Leave::Stay {
                if how == Leave::End {
                    self.end_remote().await;
                }
                return None;
            }
            // Keystrokes typed while disconnected are not replayed into a
            // shell the user cannot see; the last size is kept.
            while let Ok(f) = input_rx.try_recv() {
                if let Frame::Resize { cols, rows } = f {
                    self.size = (cols, rows);
                }
            }
            let over = |code| {
                Some(Done {
                    code,
                    session_over: true,
                })
            };
            match self.attach_once(&mut input_rx, &mut leave_rx).await {
                Ended::Exited(code) => {
                    // A new session's output starts at 0.
                    self.write_offset(0);
                    return over(code);
                }
                Ended::Left => return None,
                // The pane itself ended the session (a signal): it is over,
                // and the pane offers a restart.
                Ended::EndedHere => {
                    self.write_offset(0);
                    return over(-1);
                }
                Ended::Dropped {
                    code,
                    attached,
                    stderr,
                } => {
                    if !attached && code == Some(EXIT_COMMAND_NOT_FOUND) {
                        // No helper on the host (yet): install it once, then
                        // attach again; a failed install stops the pane.
                        if installed_once {
                            self.note(&format!(
                                "AgentMux's helper still is not runnable on {} ({}).",
                                self.conn,
                                helper_path()
                            ))
                            .await;
                            return over(EXIT_COMMAND_NOT_FOUND);
                        }
                        installed_once = true;
                        match self.install_helper().await {
                            Ok(()) => continue,
                            Err(line) => {
                                self.note(&line).await;
                                return over(EXIT_COMMAND_NOT_FOUND);
                            }
                        }
                    }
                    // A refused login (the user cancelled a password, a key
                    // was not accepted, a host key changed) stops the pane:
                    // retrying would ask the user again every 30 s, or fail
                    // for ever. The session, if any, stays on the host for
                    // the pane's restart to reattach to.
                    if !attached && code == Some(EXIT_SSH_FAILED) && login_refused(&stderr) {
                        let why = stderr
                            .lines()
                            .rev()
                            .find(|l| login_refused(l))
                            .unwrap_or("")
                            .trim();
                        self.note(&format!(
                            "Could not log in to {} ({why}). Press to try again.",
                            self.conn
                        ))
                        .await;
                        stopped(Done {
                            code: EXIT_SSH_FAILED,
                            session_over: false,
                        });
                        // Stay until the pane leaves: closing it still ends
                        // the session (one `end`, best effort, any prompt to
                        // the user), as for any durable pane. A restart
                        // replaces this run: its leave sender goes, and this
                        // returns.
                        loop {
                            if leave_rx.changed().await.is_err() {
                                return None;
                            }
                            let how = *leave_rx.borrow();
                            match how {
                                Leave::Stay => {}
                                Leave::Detach => return None,
                                Leave::End => {
                                    self.end_remote().await;
                                    return None;
                                }
                            }
                        }
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
                        // The top of the loop sees the leave and acts on it
                        // (ending the session if asked), connected or not.
                        _ = leave_rx.changed() => {}
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
        use crate::backend::remote::status;
        let mut expected = self.read_offset();
        let remote = attach_command(&self.session, expected, self.size.0, self.size.1);
        let mut cmd = self.ssh_command(&remote);
        cmd.stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
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
        let mut ended_here = false;
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
                    let mut store_failed = false;
                    for frame in frames {
                        match frame {
                            Frame::Hello { created, .. } => {
                                attached = true;
                                status::pane_started(self.broker.as_deref(), &self.conn);
                                // New panes on this host are durable by default.
                                crate::backend::remote::helper_hosts::remember(&self.conn);
                                if created && expected > 0 {
                                    self.note("The previous session on this host is gone (the host restarted or it was ended); this is a new one.").await;
                                    expected = 0;
                                    self.write_offset(0);
                                }
                            }
                            Frame::Output { offset, data } => match accept(expected, offset, data.len()) {
                                Accept::Seen => {}
                                // The offset moves only past bytes that were
                                // stored: a failed write drops this ssh, and the
                                // reattach asks for them again.
                                Accept::Append { skip, next } => {
                                    if !self.append(data[skip..].to_vec()).await {
                                        store_failed = true;
                                        break;
                                    }
                                    expected = next;
                                    self.write_offset(next);
                                }
                                Accept::Gap { from, to, next } => {
                                    self.note(&format!("{} bytes of output were lost while detached.", to - from)).await;
                                    if !self.append(data).await {
                                        store_failed = true;
                                        break;
                                    }
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
                    if store_failed {
                        break None;
                    }
                }
                frame = input_rx.recv() => {
                    let Some(frame) = frame else {
                        // stop/shutdown drop the input after asking to leave:
                        // keep what they asked (End on close), never assume.
                        let how = *leave_rx.borrow();
                        break Some(if how == Leave::Stay { Leave::Detach } else { how });
                    };
                    if let Frame::Resize { cols, rows } = frame {
                        self.size = (cols, rows);
                    }
                    let end = frame == Frame::End;
                    if stdin.write_all(&frame.encode()).await.is_err() {
                        break None;
                    }
                    if end {
                        ended_here = true;
                        break Some(Leave::End);
                    }
                }
                _ = ping.tick() => {
                    // Only once attached: before the helper's Hello, ssh may
                    // be waiting on the user to answer a prompt (askpass),
                    // which takes as long as it takes.
                    if attached && last_heard.elapsed() > STALLED_AFTER {
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
            return if ended_here {
                Ended::EndedHere
            } else {
                Ended::Left
            };
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

    /// End the session on the host with one short `ssh` (`agentmux-remote
    /// end`): a pane closed while disconnected must not leave its shell
    /// running there with nothing able to reach it. Best effort, bounded.
    async fn end_remote(&self) {
        let remote = format!("{} end --session {}", helper_path(), self.session);
        let mut cmd = self.ssh_command(&remote);
        cmd.stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        if let Ok(mut child) = cmd.spawn() {
            let _ = tokio::time::timeout(Duration::from_secs(10), child.wait()).await;
        }
    }

    fn ssh_command(&self, remote: &str) -> tokio::process::Command {
        self.host.command(remote)
    }

    /// Put this version's helper on the host (`helper_install::ensure`),
    /// saying so in the pane, once the user has said yes
    /// (`helper_consent`). `Err` is the line for the pane.
    async fn install_helper(&self) -> Result<(), String> {
        let plain = "Turn off \"Keep Session Alive\" for a plain SSH terminal.";
        if let Err(why) =
            crate::backend::remote::helper_consent::allow_install(&self.conn, Some(&self.block_id)).await
        {
            return Err(format!(
                "This pane can't stay alive without AgentMux's helper on {}: {why}. {plain}",
                self.conn
            ));
        }
        crate::backend::remote::helper_install::ensure(&self.host, &self.conn, &self.session, |size| {
            self.note_owned(format!(
                "Installing AgentMux's helper on {} ({} KB, in ~/.agentmux-remote) so this pane can stay alive…",
                self.conn,
                size / 1024
            ))
        })
        .await
        .map_err(|e| format!("Could not install AgentMux's helper on {}: {e}. {plain}", self.conn))
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
    async fn append(&self, data: Vec<u8>) -> bool {
        let (Some(broker), Some(fs)) = (self.broker.clone(), self.filestore.clone()) else {
            return false;
        };
        let block_id = self.block_id.clone();
        tokio::task::spawn_blocking(move || append_checked(&broker, &fs, &block_id, &data))
            .await
            .unwrap_or(false)
    }

    /// [`Self::note`] with an owned line (for a closure's future).
    async fn note_owned(&self, text: String) {
        self.note(&text).await
    }

    /// A line from AgentMux in the pane, dimmed, on its own line.
    async fn note(&self, text: &str) {
        let _ = self
            .append(format!("\r\n\x1b[2m[{text}]\x1b[0m\r\n").into_bytes())
            .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::remote::conn::SshDest;

    #[test]
    fn only_a_helper_session_id_reaches_a_command_line() {
        assert!(valid_session_id("amx-0123abcd") && valid_session_id("a_b-C"));
        for bad in [
            "",
            "bad/id; touch /tmp/pwn",
            "a b",
            "$(id)",
            &"x".repeat(65),
        ] {
            assert!(!valid_session_id(bad), "{bad:?}");
        }
    }

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
            host: HostSsh {
                dest: SshDest {
                    destination: "fakehost".to_string(),
                    port: None,
                },
                ssh_path,
                control_dir: None,
                env: Vec::new(),
            },
            session: "amx-test".to_string(),
            size: (80, 24),
            broker: Some(state.broker.clone()),
            filestore: Some(state.filestore.clone()),
            _askpass_grant: None,
        };
        let (_input_tx, input_rx) = mpsc::unbounded_channel();
        let (_leave_tx, leave_rx) = watch::channel(Leave::Stay);
        let ended = tokio::time::timeout(
            Duration::from_secs(30),
            run.run(input_rx, leave_rx, Box::new(|_| {})),
        )
        .await
        .expect("the run ends");
        assert_eq!(
            ended,
            Some(Done {
                code: 0,
                session_over: true
            }),
            "the session's own end ends the run"
        );

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
        // Over, so the next session starts from 0 (no "the previous session
        // is gone" for a plain `exit`).
        assert_eq!(offset, Some(serde_json::json!(0)));
    }

    /// Every attach fails (the host is unreachable); `end` is logged.
    const FAKE_SSH_DOWN: &str = r#"
import os, sys
remote = sys.argv[-1].split()
with open(os.environ['FAKE_SSH_LOG2'], 'ab') as log:
    log.write((' '.join(remote[1:3]) + '\n').encode())
sys.exit(0 if remote[1] == 'end' else 255)
"#;

    /// A pane closed while its host is unreachable still ends its session
    /// there (one `agentmux-remote end`), rather than leaving the shell
    /// running with nothing able to reach it.
    #[tokio::test]
    async fn closing_while_disconnected_still_ends_the_session() {
        let Some(python) = python() else {
            eprintln!("skipped: no python to stand in for ssh");
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("fake_ssh_down.py");
        std::fs::write(&script, FAKE_SSH_DOWN).unwrap();
        let ssh_path = if cfg!(windows) {
            let cmd = dir.path().join("fake_ssh_down.cmd");
            std::fs::write(
                &cmd,
                format!("@\"{}\" \"{}\" %*\r\n", python.display(), script.display()),
            )
            .unwrap();
            cmd
        } else {
            let sh = dir.path().join("fake_ssh_down");
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
        let log = dir.path().join("log2");
        std::env::set_var("FAKE_SSH_LOG2", &log);

        let state = crate::server::tests::test_state();
        let run = Run {
            block_id: "durable-down-block".to_string(),
            conn: "downhost".to_string(),
            host: HostSsh {
                dest: SshDest {
                    destination: "downhost".to_string(),
                    port: None,
                },
                ssh_path,
                control_dir: None,
                env: Vec::new(),
            },
            session: "amx-down".to_string(),
            size: (80, 24),
            broker: Some(state.broker.clone()),
            filestore: Some(state.filestore.clone()),
            _askpass_grant: None,
        };
        let (_input_tx, input_rx) = mpsc::unbounded_channel();
        let (leave_tx, leave_rx) = watch::channel(Leave::Stay);
        let task = tokio::spawn(run.run(input_rx, leave_rx, Box::new(|_| {})));
        // Let it fail an attach and start backing off, then close the pane.
        tokio::time::sleep(Duration::from_millis(1500)).await;
        leave_tx.send(Leave::End).unwrap();
        let ended = tokio::time::timeout(Duration::from_secs(20), task)
            .await
            .expect("the run ends")
            .unwrap();
        assert_eq!(ended, None, "closed, not the session's own end");
        let calls = std::fs::read_to_string(&log).unwrap();
        assert!(calls.lines().any(|l| l.starts_with("attach")), "{calls:?}");
        assert_eq!(
            calls.lines().last(),
            Some("end --session"),
            "the last ssh ends the session: {calls:?}"
        );
    }

    /// A host without the helper: attach fails with 127 until the helper is
    /// installed; the probe, upload and install steps behave as on a real
    /// host, the install step checking the uploaded file's hash itself.
    const FAKE_SSH_FRESH_HOST: &str = r#"
import hashlib, os, struct, sys
remote = sys.argv[-1]
d = os.environ['FAKE_HOST_DIR']
log = open(os.path.join(d, 'log'), 'ab', buffering=0)
helper = os.path.join(d, 'helper')
if remote.startswith('sh -c') and 'uname -sm' in remote:
    log.write(b'probe\n')
    sys.stdout.write('Linux x86_64\n')
elif remote.startswith('sh -c') and 'cat >' in remote:
    log.write(b'upload\n')
    open(helper + '.tmp', 'wb').write(sys.stdin.buffer.read())
elif remote.startswith('sh -c') and 'mv -f' in remote:
    log.write(b'install\n')
    got = hashlib.sha256(open(helper + '.tmp', 'rb').read()).hexdigest()
    if got in remote:
        os.replace(helper + '.tmp', helper)
        sys.stdout.write('ok\n')
    else:
        sys.stdout.write('mismatch ' + got + '\n')
elif ' attach ' in remote:
    if not os.path.exists(helper):
        log.write(b'attach-missing\n')
        sys.exit(127)
    log.write(b'attach\n')
    out = sys.stdout.buffer
    def frame(kind, payload):
        out.write(struct.pack('>BI', kind, len(payload)) + payload)
        out.flush()
    frame(14, struct.pack('>Q', 0) + bytes([1]) + b'amx-fresh')
    frame(10, struct.pack('>Q', 0) + b'ready\r\n')
    frame(12, struct.pack('>i', 0))
    import time; time.sleep(5)
"#;

    /// A first durable pane on a host without the helper installs it (probe,
    /// upload, hash-checked install) and then attaches.
    #[tokio::test]
    async fn a_host_without_the_helper_gets_it_installed_then_attaches() {
        let Some(python) = python() else {
            eprintln!("skipped: no python to stand in for ssh");
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("fake_ssh_fresh.py");
        std::fs::write(&script, FAKE_SSH_FRESH_HOST).unwrap();
        let ssh_path = if cfg!(windows) {
            let cmd = dir.path().join("fake_ssh_fresh.cmd");
            std::fs::write(
                &cmd,
                format!("@\"{}\" \"{}\" %*\r\n", python.display(), script.display()),
            )
            .unwrap();
            cmd
        } else {
            let sh = dir.path().join("fake_ssh_fresh");
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
        std::env::set_var("FAKE_HOST_DIR", dir.path());
        // The user says Install when asked.
        crate::backend::remote::helper_consent::install_answering_yes();
        // The "release" build, from a local folder.
        let builds = dir.path().join("builds");
        std::fs::create_dir_all(&builds).unwrap();
        let name = crate::backend::remote::helper_install::asset_name(
            env!("CARGO_PKG_VERSION"),
            "x86_64-unknown-linux-musl",
        );
        std::fs::write(builds.join(name), b"the helper").unwrap();
        std::env::set_var("AGENTMUX_REMOTE_HELPER_DIR", &builds);

        let state = crate::server::tests::test_state();
        let run = Run {
            block_id: "durable-fresh-block".to_string(),
            conn: "freshhost".to_string(),
            host: HostSsh {
                dest: SshDest {
                    destination: "freshhost".to_string(),
                    port: None,
                },
                ssh_path,
                control_dir: None,
                env: Vec::new(),
            },
            session: "amx-fresh".to_string(),
            size: (80, 24),
            broker: Some(state.broker.clone()),
            filestore: Some(state.filestore.clone()),
            _askpass_grant: None,
        };
        let (_input_tx, input_rx) = mpsc::unbounded_channel();
        let (_leave_tx, leave_rx) = watch::channel(Leave::Stay);
        let ended = tokio::time::timeout(
            Duration::from_secs(30),
            run.run(input_rx, leave_rx, Box::new(|_| {})),
        )
        .await
        .expect("the run ends");
        std::env::remove_var("AGENTMUX_REMOTE_HELPER_DIR");
        assert_eq!(
            ended,
            Some(Done {
                code: 0,
                session_over: true
            })
        );
        let log = std::fs::read_to_string(dir.path().join("log")).unwrap();
        assert_eq!(log, "attach-missing\nprobe\nupload\ninstall\nattach\n");
        let term = state
            .filestore
            .read_file("durable-fresh-block", "term")
            .unwrap()
            .unwrap();
        let text = String::from_utf8_lossy(&term).into_owned();
        assert!(
            text.contains("Installing AgentMux's helper on freshhost") && text.contains("ready"),
            "{text:?}"
        );
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
        assert!(wants(&meta("area54", Some(true)), None));
        assert!(!wants(&meta("area54", Some(false)), None));
        assert!(!wants(&meta("local", Some(true)), None));
        assert!(!wants(&meta("wsl://Ubuntu", Some(true)), None));

        // The connection's setting, then the global one, under the pane's.
        let mut config = crate::backend::wconfig::FullConfigType::default();
        config.settings.term_durable = Some(false);
        config.connections.insert(
            "durable-test-host".into(),
            crate::backend::wconfig::ConnKeywords {
                term_durable: Some(true),
                ..Default::default()
            },
        );
        assert!(wants(&meta("durable-test-host", None), Some(&config)));
        assert!(!wants(
            &meta("durable-test-host", Some(false)),
            Some(&config)
        ));
        assert!(!wants(&meta("durable-test-other", None), Some(&config)));

        // An agent's SSH shell: never, not even when asked or by default.
        let mut agent = meta("durable-test-host", Some(true));
        agent.insert(
            crate::backend::remote::askpass::META_KEY_AGENT_BLOCK.into(),
            serde_json::json!("agent-block"),
        );
        assert!(!wants(&agent, Some(&config)));
    }

    #[test]
    fn durable_is_the_pane_then_the_connection_then_global_then_the_helper() {
        let never = || panic!("asked the host only when nothing is set");
        assert!(resolve_durable(Some(true), Some(false), Some(false), never));
        assert!(!resolve_durable(None, Some(false), Some(true), never));
        assert!(resolve_durable(None, None, Some(true), never));
        assert!(resolve_durable(None, None, None, || true));
        assert!(!resolve_durable(None, None, None, || false));
    }

    #[test]
    fn a_refused_login_is_told_from_a_dropped_link() {
        assert!(login_refused(
            "user@box: Permission denied (publickey,password)."
        ));
        assert!(login_refused("Host key verification failed."));
        assert!(login_refused(
            "@    WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED!     @"
        ));
        assert!(!login_refused(
            "ssh: connect to host box port 22: Connection timed out"
        ));
        assert!(!login_refused("Connection reset by peer"));
        assert!(!login_refused(""));
    }

    /// The host refuses the login (stderr is the point); each run logs its
    /// helper command (`attach` or `end`).
    const FAKE_SSH_REFUSED: &str = r#"
import os, sys
with open(os.environ['FAKE_SSH_LOG3'], 'ab') as log:
    log.write((sys.argv[-1].split()[1] + '\n').encode())
sys.stderr.write('user@fakehost: Permission denied (publickey).\n')
sys.exit(255)
"#;

    /// A refused login stops the pane at once with the reason, rather than
    /// asking again every 30 s, and keeps the session id: the session may
    /// still be on the host for the pane's restart to reattach to. Closing
    /// the pane then still ends the session there.
    #[tokio::test]
    async fn a_refused_login_stops_the_pane_and_keeps_its_session() {
        let Some(python) = python() else {
            eprintln!("skipped: no python to stand in for ssh");
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("fake_ssh_refused.py");
        std::fs::write(&script, FAKE_SSH_REFUSED).unwrap();
        let ssh_path = if cfg!(windows) {
            let cmd = dir.path().join("fake_ssh_refused.cmd");
            std::fs::write(
                &cmd,
                format!("@\"{}\" \"{}\" %*\r\n", python.display(), script.display()),
            )
            .unwrap();
            cmd
        } else {
            let sh = dir.path().join("fake_ssh_refused");
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
        std::env::set_var("FAKE_SSH_LOG3", dir.path().join("log"));

        let state = crate::server::tests::test_state();
        let block = "durable-refused-block";
        let run = Run {
            block_id: block.to_string(),
            conn: "fakehost".to_string(),
            host: HostSsh {
                dest: SshDest {
                    destination: "fakehost".to_string(),
                    port: None,
                },
                ssh_path,
                control_dir: None,
                env: Vec::new(),
            },
            session: "amx-refused".to_string(),
            size: (80, 24),
            broker: Some(state.broker.clone()),
            filestore: Some(state.filestore.clone()),
            _askpass_grant: None,
        };
        let (_input_tx, input_rx) = mpsc::unbounded_channel();
        let (leave_tx, leave_rx) = watch::channel(Leave::Stay);
        let (stopped_tx, stopped_rx) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(run.run(
            input_rx,
            leave_rx,
            Box::new(move |done| {
                let _ = stopped_tx.send(done);
            }),
        ));
        // Well inside the first backoff: no retry was waited for.
        let stopped = tokio::time::timeout(Duration::from_secs(15), stopped_rx)
            .await
            .expect("the pane is shown done")
            .unwrap();
        assert_eq!(
            stopped,
            Done {
                code: EXIT_SSH_FAILED,
                session_over: false
            }
        );
        let log = std::fs::read_to_string(dir.path().join("log")).unwrap();
        assert_eq!(log, "attach\n", "asked once, not again");

        // Closing the pane: one `end` for the session it kept.
        leave_tx.send(Leave::End).unwrap();
        let ended = tokio::time::timeout(Duration::from_secs(15), task)
            .await
            .expect("the run ends")
            .unwrap();
        assert_eq!(ended, None);
        let log = std::fs::read_to_string(dir.path().join("log")).unwrap();
        assert_eq!(log, "attach\nend\n");
        let term = state.filestore.read_file(block, "term").unwrap().unwrap();
        let text = String::from_utf8_lossy(&term).into_owned();
        assert!(
            text.contains("Could not log in to fakehost")
                && text.contains("Permission denied (publickey)"),
            "{text:?}"
        );
    }
}
