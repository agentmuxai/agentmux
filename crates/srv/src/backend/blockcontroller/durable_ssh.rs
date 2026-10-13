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

use super::durable_mode::{mode, wants, Mode};

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
/// How long an `ssh` whose output ended has to exit before it is killed.
const EXIT_GRACE: Duration = Duration::from_secs(5);
/// Tries a pane durable only by default makes at a host it has never reached,
/// and the helper never ran on, before it becomes a plain SSH terminal.
const FIRST_REACH_TRIES: u32 = 3;

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

/// The host's shell is Windows' (`cmd` or PowerShell), which can't run the
/// helper: it answered the attach command that it doesn't know it.
pub fn windows_shell_refused(stderr: &str) -> bool {
    [
        "is not recognized as an internal or external command",
        "is not recognized as a name of a cmdlet",
        "is not recognized as the name of a cmdlet",
    ]
    .iter()
    .any(|p| stderr.contains(p))
}

/// The host's answer to `cmd /c echo %OS%`: Windows, not Windows, or `None`
/// when ssh itself failed (exit 255, or no exit code) and the host never
/// answered.
fn windows_answer(out: &crate::backend::remote::host::HostOutput) -> Option<bool> {
    if out.stdout.contains("Windows_NT") {
        return Some(true);
    }
    match out.code {
        Some(code) if code != EXIT_SSH_FAILED => Some(false),
        _ => None,
    }
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
    /// Restarts the pane as a plain SSH terminal, when the user declines the
    /// helper ([`Fallback`]).
    fallback: Option<Fallback>,
    inner: Mutex<Inner>,
}

/// Restarts a pane through `resync_controller`, which then picks the plain
/// shell controller because [`wants`] has turned false. Built by
/// `resync_controller` itself, which holds everything a restart needs.
/// False when it did not restart: the pane would still be durable (so a
/// restart would only come back here), or the block is gone.
pub type Fallback = Arc<dyn Fn() -> bool + Send + Sync>;

impl DurableSshController {
    pub fn new(
        block_id: String,
        broker: Option<Arc<mps::Broker>>,
        event_bus: Option<Arc<EventBus>>,
        mstore: Option<Arc<Store>>,
        filestore: Option<Arc<FileStore>>,
        auth_key: String,
        config: Option<Arc<crate::backend::wconfig::ConfigState>>,
        fallback: Option<Fallback>,
    ) -> Self {
        Self {
            block_id,
            broker,
            event_bus,
            mstore,
            filestore,
            auth_key,
            config,
            fallback,
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
            let update = obj::MuxObjUpdate {
                updatetype: "update".into(),
                otype: "block".into(),
                oid: self.block_id.clone(),
                obj: Some(obj::mux_obj_to_value(&block)),
            };
            bus.broadcast_event(&crate::backend::eventbus::EventBus::typed_ws_event::<crate::backend::mps_events::MuxObjUpdated>(&update, oref.clone()));
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
            plain: Arc::new(AtomicBool::new(false)),
            by_default: mode(&block_meta, self.config.as_ref().map(|c| c.get_full_config()).as_deref()) == Mode::Default,
        };
        let plain = run.plain.clone();
        let fallback = self.fallback.clone();
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
            } else if plain.load(Ordering::SeqCst) {
                match fallback {
                    Some(restart) if restart() => {}
                    _ => show_done(
                        this.as_ref(),
                        Done {
                            code: EXIT_COMMAND_NOT_FOUND,
                            session_over: true,
                        },
                    ),
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
    broker.publish_event::<crate::backend::mps_events::BlockFile>(&event_data, vec![format!("block:{block_id}")], 0);
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
    /// Set when the run ends because the helper isn't to be had (the user
    /// declined it, or durable was only the default and it didn't install):
    /// the pane becomes a plain SSH terminal.
    plain: Arc<AtomicBool>,
    /// Durable only by default ([`Mode::Default`]): without the helper the
    /// pane becomes a plain terminal rather than stopping.
    by_default: bool,
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
        let mut ever_attached = false;
        // Whether the host has answered if it is Windows (asked until it has).
        let mut platform_known = false;
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
            match self.attach_once(&mut input_rx, &mut leave_rx, noted_drop).await {
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
                    // A Windows host: the helper doesn't run there, so the
                    // pane is a plain SSH terminal (remote terminals spec
                    // §6.4).
                    // Its English answer says so at once; in another language
                    // (localised, and not POSIX's 127) the host is asked,
                    // until it answers (a failed ask is asked again).
                    let windows = !attached
                        && (windows_shell_refused(&stderr)
                            || (!platform_known
                                && code != Some(EXIT_COMMAND_NOT_FOUND)
                                && code != Some(EXIT_SSH_FAILED)
                                && match self.host_is_windows().await {
                                    Some(answer) => {
                                        platform_known = true;
                                        answer
                                    }
                                    None => false,
                                }));
                    if windows {
                        crate::backend::remote::helper_consent::note_unsupported(&self.conn);
                        // A record saying the helper is installed here is
                        // stale (it can't run on Windows), and would keep the
                        // pane durable (wants), as for a declined helper.
                        if let Err(e) = crate::backend::remote::helper_hosts::forget_helper(&self.conn) {
                            tracing::warn!(connection = %self.conn, error = %e, "could not forget the helper record");
                        }
                        self.note(&format!(
                            "AgentMux's helper doesn't run on Windows hosts like {} yet, so this is a plain SSH terminal: it ends if the connection drops.",
                            self.conn
                        ))
                        .await;
                        self.plain.store(true, Ordering::SeqCst);
                        return None;
                    }
                    if !attached && code == Some(EXIT_COMMAND_NOT_FOUND) {
                        // No helper on the host (yet): install it once, then
                        // attach again. One that still isn't there after
                        // installing it failed the same way.
                        let why = if installed_once {
                            format!("it still isn't runnable after installing it ({})", helper_path())
                        } else {
                            installed_once = true;
                            match self.install_helper().await {
                                Ok(()) => continue,
                                Err(why) => why,
                            }
                        };
                        let declined = crate::backend::remote::helper_consent::plain_instead(&self.conn);
                        // Durable was asked for and the user didn't decline
                        // the helper, but it isn't there: the pane stops, and
                        // says how to get a plain terminal.
                        if !declined && !self.by_default {
                            self.note(&format!(
                                "This pane can't stay alive without AgentMux's helper on {}: {why}. For a plain SSH terminal, set \"Keep sessions alive\" to Off for this host in Connectors > Remotes.",
                                self.conn
                            ))
                            .await;
                            return over(EXIT_COMMAND_NOT_FOUND);
                        }
                        // The user doesn't want the helper here, or durable was only the
                        // default and it didn't install: the pane falls back to a plain SSH
                        // terminal.
                        let mut line = format!(
                            "AgentMux's helper isn't on {}, so this is a plain SSH terminal: it ends if the connection drops.",
                            self.conn
                        );
                        if !declined {
                            // Not durable by default on this host again until srv restarts.
                            crate::backend::remote::helper_consent::note_not_installed(&self.conn);
                            line.push_str(&format!(" Why: {why}."));
                        }
                        self.note(&line).await;
                        // The helper was just found missing here: a record
                        // saying it's installed is stale, and would keep the
                        // pane durable (wants), so the restart would come
                        // straight back.
                        if let Err(e) = crate::backend::remote::helper_hosts::forget_helper(&self.conn) {
                            tracing::warn!(connection = %self.conn, error = %e, "could not forget the missing helper");
                        }
                        self.plain.store(true, Ordering::SeqCst);
                        return None;
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
                        let line = format!("Could not log in to {} ({why}). Press to try again.", self.conn);
                        return self.wait_for_user(&line, &mut leave_rx, stopped).await;
                    }
                    // The user disconnected this host (Remotes, or the pane's
                    // connection menu): logging back in on our own would undo
                    // it, and for a password ask again at once. The session
                    // stays on the host for the pane's restart to reattach to.
                    if crate::backend::remote::link::held(&self.conn) {
                        let line = crate::backend::remote::link::disconnected_note(&self.conn);
                        return self.wait_for_user(&line, &mut leave_rx, stopped).await;
                    }
                    // A first terminal, durable only by default, on a host
                    // it never reached and the helper never ran on (a
                    // mistyped name, a host that's down): no session can be
                    // waiting there, so after a few tries it becomes a plain
                    // SSH terminal, which shows ssh's error, as before
                    // durable was the default. A host the helper ran on
                    // keeps trying: its session may be there.
                    ever_attached |= attached;
                    if self.by_default
                        && !ever_attached
                        && code == Some(EXIT_SSH_FAILED)
                        && attempt + 1 >= FIRST_REACH_TRIES
                        && !crate::backend::remote::helper_hosts::known(&self.conn)
                    {
                        let why = stderr.lines().last().unwrap_or("").trim();
                        self.note(&format!(
                            "Could not reach {} ({why}), so this is a plain SSH terminal.",
                            self.conn
                        ))
                        .await;
                        self.plain.store(true, Ordering::SeqCst);
                        return None;
                    }
                    if attached {
                        attempt = 0;
                        noted_drop = false;
                        // Any install so far took: a helper missing from now
                        // on was removed (Remotes' Remove helper), and is
                        // installed again rather than stopping the pane.
                        installed_once = false;
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

    /// Say `line`, show the pane done without ending the session (a restart
    /// reattaches to it), and stay until the pane leaves: closing it still
    /// ends the session (one `end`, best effort, any prompt to the user), as
    /// for any durable pane. A restart replaces this run: its leave sender
    /// goes, and this returns.
    async fn wait_for_user(
        &mut self,
        line: &str,
        leave_rx: &mut watch::Receiver<Leave>,
        stopped: Box<dyn FnOnce(Done) + Send>,
    ) -> Option<Done> {
        self.note(line).await;
        stopped(Done {
            code: EXIT_SSH_FAILED,
            session_over: false,
        });
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

    /// One `ssh` to the session, until it drops or the pane leaves.
    /// `reconnecting`: the pane said it lost the session, so it says when it
    /// has it again.
    async fn attach_once(
        &mut self,
        input_rx: &mut mpsc::UnboundedReceiver<Frame>,
        leave_rx: &mut watch::Receiver<Leave>,
        reconnecting: bool,
    ) -> Ended {
        use crate::backend::remote::link;
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
        let mut output_ended = false;
        let mut last_heard = tokio::time::Instant::now();
        let mut ping = tokio::time::interval(PING_EVERY);
        ping.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let ended = loop {
            tokio::select! {
                read = stdout.read(&mut buf) => {
                    let n = match read {
                        Ok(0) | Err(_) => {
                            output_ended = true;
                            break None;
                        }
                        Ok(n) => n,
                    };
                    last_heard = tokio::time::Instant::now();
                    let Ok(frames) = decoder.push(&buf[..n]) else { break None };
                    let mut exited: Option<i32> = None;
                    let mut store_failed = false;
                    for frame in frames {
                        match frame {
                            Frame::Hello { created, .. } => {
                                attached = true;
                                // The helper answered: a live login.
                                link::pane_started(&self.conn, true);
                                // New panes on this host are durable by default.
                                crate::backend::remote::helper_hosts::remember(&self.conn);
                                if created && expected > 0 {
                                    self.note("The previous session on this host is gone (the host restarted or it was ended); this is a new one.").await;
                                    expected = 0;
                                    self.write_offset(0);
                                } else if reconnecting {
                                    // "reconnecting…" was the pane's last word
                                    // until now (#4513).
                                    self.note(&format!("Reconnected to {}.", self.conn)).await;
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
                                link::pane_ended(&self.conn, None);
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
            link::pane_ended(&self.conn, None);
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
            // Closed while this ssh was still trying to reach the session
            // (the host down, a reconnect under way): the End never got
            // there, and the session would run on with nothing able to
            // reach it. End it the way a pane closed while disconnected does.
            if how == Leave::End && !attached {
                self.end_remote().await;
            }
            return if ended_here {
                Ended::EndedHere
            } else {
                Ended::Left
            };
        }
        // ssh closes its output a moment before it exits: wait for its exit
        // code (127: no helper, 255: its own failure) before killing it,
        // which lost the code, so a host without the helper reconnected for
        // ever instead of installing it. A stalled ssh is killed at once.
        let exited = if output_ended {
            tokio::time::timeout(EXIT_GRACE, child.wait()).await.ok().and_then(Result::ok)
        } else {
            None
        };
        let status = match exited {
            Some(status) => Some(status),
            None => {
                let _ = child.kill().await;
                child.wait().await.ok()
            }
        };
        let code = status.and_then(|s| s.code());
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

    /// Whether the host is Windows, asked in a way no language changes: from
    /// `cmd` or PowerShell, `cmd /c echo %OS%` prints `Windows_NT`; a POSIX
    /// host has no `cmd`. `None` when there was no answer from the host
    /// (ssh failed or timed out), so it can be asked again.
    async fn host_is_windows(&self) -> Option<bool> {
        let out = self
            .host
            .run("cmd /c echo %OS%", None, Duration::from_secs(15))
            .await
            .ok()?;
        windows_answer(&out)
    }

    fn ssh_command(&self, remote: &str) -> tokio::process::Command {
        self.host.command(remote)
    }

    /// Put this version's helper on the host (`helper_install::ensure`),
    /// saying so in the pane, once the user has said yes
    /// (`helper_consent`). `Err` says why not.
    async fn install_helper(&self) -> Result<(), String> {
        crate::backend::remote::helper_consent::allow_install(&self.conn, Some(&self.block_id)).await?;
        crate::backend::remote::helper_install::ensure(&self.host, &self.conn, &self.session, |size| {
            self.note_owned(format!(
                "Installing AgentMux's helper on {} ({} KB, in ~/.agentmux-remote) so this pane can stay alive…",
                self.conn,
                size / 1024
            ))
        })
        .await
        .map_err(|e| format!("installing it failed: {e}"))
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

    /// A line from AgentMux in the pane (`pane_note`).
    async fn note(&self, text: &str) {
        let _ = self.append(super::pane_note(text)).await;
    }
}

#[cfg(test)]
#[path = "durable_ssh_tests.rs"]
mod tests;
