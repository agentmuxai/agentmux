// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! First-class controller shell for a Codex App Server process.
//!
//! The protocol implementation lives in [`super::app_server`] and
//! [`super::app_server_protocol`]. This module owns the AgentMux controller
//! lifecycle: command construction, process/thread startup, turn dispatch,
//! block output publication, and shutdown. Codex remains opt-in until the
//! provider registry flips its controller type.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use tokio::process::Command;

use super::app_server::{AppServerExit, AppServerIncoming, AppServerLimits, AppServerProcess};
use super::app_server_protocol::{
    CodexAppServerEvent, CodexAppServerProtocolError, CodexAppServerSession, ThreadStartOptions,
};
use super::core;
use super::health::TurnActivityTracker;
use super::{
    BlockControllerRuntimeStatus, BlockInputUnion, Controller, STATUS_DONE, STATUS_INIT,
    STATUS_RUNNING,
};
use crate::backend::eventbus::EventBus;
use crate::backend::storage::filestore::FileStore;
use crate::backend::storage::store::Store;
use crate::backend::wps;

const INITIALIZE_TIMEOUT: Duration = Duration::from_secs(10);
const OPERATION_TIMEOUT: Duration = Duration::from_secs(10);

struct AppServerInner {
    proc_status: String,
    proc_exit_code: i32,
    status_version: i32,
    process: Option<Arc<AppServerProcess>>,
    session: Option<Arc<CodexAppServerSession>>,
    pending_messages: VecDeque<String>,
}

/// Controller for one isolated `codex app-server --listen stdio://` child.
pub struct AppServerController {
    tab_id: String,
    block_id: String,
    inner: Arc<Mutex<AppServerInner>>,
    broker: Option<Arc<wps::Broker>>,
    event_bus: Option<Arc<EventBus>>,
    wstore: Option<Arc<Store>>,
    filestore: Option<Arc<FileStore>>,
    health_monitor: Arc<TurnActivityTracker>,
    self_ref: Mutex<Option<Weak<Self>>>,
}

impl AppServerController {
    pub fn new(
        tab_id: String,
        block_id: String,
        broker: Option<Arc<wps::Broker>>,
        event_bus: Option<Arc<EventBus>>,
        wstore: Option<Arc<Store>>,
        filestore: Option<Arc<FileStore>>,
    ) -> Self {
        Self {
            tab_id,
            block_id: block_id.clone(),
            inner: Arc::new(Mutex::new(AppServerInner {
                proc_status: STATUS_INIT.to_string(),
                proc_exit_code: 0,
                status_version: 0,
                process: None,
                session: None,
                pending_messages: VecDeque::new(),
            })),
            broker,
            event_bus,
            wstore,
            filestore,
            health_monitor: Arc::new(TurnActivityTracker::new(block_id)),
            self_ref: Mutex::new(None),
        }
    }

    pub fn set_self_ref(self: &Arc<Self>) {
        *self.self_ref.lock().unwrap() = Some(Arc::downgrade(self));
    }

    fn set_status(&self, status: &str) {
        // Scoped: `publish_status` -> `get_runtime_status` re-locks `self.inner`.
        // Holding this guard across that call deadlocks (std::sync::Mutex isn't
        // reentrant) — masked in the existing unit tests because they all
        // construct the controller with `broker: None`, which short-circuits
        // `publish_status` before it ever re-locks. Any real controller (always
        // built with `Some(broker)`) would hang the instant `start()` calls
        // `set_status(STATUS_RUNNING)`.
        {
            let mut inner = self.inner.lock().unwrap();
            inner.proc_status = status.to_string();
            inner.status_version += 1;
        }
        self.publish_status();
    }

    fn publish_status(&self) {
        if let Some(broker) = &self.broker {
            super::publish_controller_status(broker, &self.get_runtime_status());
        }
    }

    fn command_from_meta(
        &self,
        block_meta: &crate::backend::obj::MetaMapType,
    ) -> Result<Command, String> {
        let executable = crate::backend::obj::meta_get_string(block_meta, "cmd", "codex");
        let args = match block_meta.get("cmd:args") {
            Some(serde_json::Value::Array(values)) => values
                .iter()
                .filter_map(|value| value.as_str().map(str::to_string))
                .collect::<Vec<_>>(),
            _ => vec![
                "app-server".to_string(),
                "--listen".to_string(),
                "stdio://".to_string(),
            ],
        };
        if args.is_empty() {
            return Err("App Server command must include app-server arguments".to_string());
        }
        let working_dir = crate::backend::obj::meta_get_string(block_meta, "cmd:cwd", "");
        let env_vars = match block_meta.get("cmd:env") {
            Some(serde_json::Value::Object(values)) => values
                .iter()
                .filter_map(|(key, value)| {
                    value.as_str().map(|value| (key.clone(), value.to_string()))
                })
                .collect::<HashMap<_, _>>(),
            _ => HashMap::new(),
        };
        let mut command = crate::server::cli_handlers::make_cli_cmd(&executable);
        command.args(args);
        core::apply_working_dir(&mut command, &self.block_id, &working_dir, &env_vars);
        #[cfg(windows)]
        {
            use agentmux_common::win32::CREATE_NO_WINDOW;
            command.creation_flags(CREATE_NO_WINDOW);
        }
        Ok(command)
    }

    fn publish_protocol_frame(&self, frame: serde_json::Value) {
        let line = format!("{}\n", frame);
        if let Some(broker) = &self.broker {
            broker.publish(wps::WaveEvent {
                event: "output".to_string(),
                scopes: vec![format!("block:{}", self.block_id)],
                sender: String::new(),
                persist: 1,
                data: Some(frame.clone()),
            });
        }
        if let (Some(filestore), Some(broker)) = (&self.filestore, &self.broker) {
            super::shell::handle_append_block_file(
                broker,
                &self.block_id,
                "output",
                line.as_bytes(),
                Some(filestore),
                None,
            );
        }
    }

    fn spawn_event_loop(
        &self,
        process: Arc<AppServerProcess>,
        session: Arc<CodexAppServerSession>,
    ) {
        let weak = self.self_ref.lock().unwrap().clone();
        tokio::spawn(async move {
            while let Some(incoming) = process.transport.next_incoming().await {
                let frame = match &incoming {
                    AppServerIncoming::Notification { method, params } => {
                        serde_json::json!({"type":"codex_app_server","method":method,"params":params})
                    }
                    AppServerIncoming::Request {
                        id: _,
                        method,
                        params,
                    } => {
                        serde_json::json!({"type":"codex_app_server_request","method":method,"params":params})
                    }
                };
                if let Some(controller) = weak.as_ref().and_then(Weak::upgrade) {
                    controller.publish_protocol_frame(frame);
                    match incoming {
                        notification @ AppServerIncoming::Notification { .. } => {
                            if let Ok(event) = session.apply_incoming(notification) {
                                if matches!(event, CodexAppServerEvent::TurnCompleted { .. }) {
                                    controller.health_monitor.set_active_turn(false);
                                    controller.set_status(STATUS_RUNNING);
                                    // Send the next queued message (if any) now that
                                    // the turn it collided with has finished — one at
                                    // a time, since only one turn can be active. Any
                                    // remainder stays queued for the NEXT completion.
                                    let next_message =
                                        controller.inner.lock().unwrap().pending_messages.pop_front();
                                    if let Some(next_message) = next_message {
                                        controller.spawn_turn(session.clone(), next_message);
                                    }
                                }
                            }
                        }
                        request @ AppServerIncoming::Request { .. } => {
                            if let Err(error) = session.accept_server_request(request) {
                                tracing::warn!(block_id = %controller.block_id, error = %error, "Codex App Server request rejected");
                            }
                        }
                    }
                }
            }
            // The transport closed (EOF/crash/shutdown). Reap the child for its
            // real exit status instead of hardcoding one, and clear the process/
            // session so a subsequent `start()` (resync) doesn't see a stale
            // `process.is_some()` and wrongly no-op on a controller that is, from
            // here on, actually done.
            let exit_code = match process.wait_for_exit().await {
                Ok(AppServerExit::Clean { code, .. })
                | Ok(AppServerExit::BeforeInitialize { code, .. })
                | Ok(AppServerExit::NonZero { code, .. }) => code.unwrap_or(1),
                _ => 1,
            };
            if let Some(controller) = weak.as_ref().and_then(Weak::upgrade) {
                controller.health_monitor.set_exited(exit_code);
                {
                    let mut inner = controller.inner.lock().unwrap();
                    inner.process = None;
                    inner.session = None;
                    inner.proc_exit_code = exit_code;
                }
                controller.set_status(STATUS_DONE);
            }
        });
    }

    fn spawn_turn(&self, session: Arc<CodexAppServerSession>, message: String) {
        let health = self.health_monitor.clone();
        let weak = self.self_ref.lock().unwrap().clone();
        health.set_active_turn(true);
        self.set_status(STATUS_RUNNING);
        // Kept alongside `message` (which `start_turn` consumes) so a
        // `TurnAlreadyActive` rejection can be requeued instead of lost.
        let requeue_message = message.clone();
        tokio::spawn(async move {
            if let Err(error) = session.start_turn(message, OPERATION_TIMEOUT).await {
                // `TurnAlreadyActive` means a DIFFERENT turn is still running —
                // this attempt (queued via `send_message`, which already returned
                // `Ok` to its caller) was rejected, but the original turn and the
                // process behind it are still very much alive. Treating that the
                // same as a real failure marked a live controller STATUS_DONE and
                // health inactive out from under its own running turn.
                let turn_already_active =
                    matches!(error, CodexAppServerProtocolError::TurnAlreadyActive(_));
                if !turn_already_active {
                    health.set_active_turn(false);
                }
                if let Some(controller) = weak.and_then(|weak| Weak::upgrade(&weak)) {
                    if turn_already_active {
                        // ReAgent P1 on PR #3215's second review: don't just log and
                        // drop it — `send_message` already told its caller `Ok`, so
                        // this is the only chance to actually deliver it. Queue it
                        // for `spawn_event_loop`'s `TurnCompleted` handler to send
                        // once the turn that's currently active finishes.
                        controller
                            .inner
                            .lock()
                            .unwrap()
                            .pending_messages
                            .push_back(requeue_message);
                    } else {
                        controller.set_status(STATUS_DONE);
                    }
                    tracing::warn!(
                        block_id = %controller.block_id,
                        error = %error,
                        turn_already_active,
                        "Codex App Server turn failed to start"
                    );
                }
            }
        });
    }

    pub fn send_message(&self, message: String) -> Result<(), String> {
        let session = {
            let mut inner = self.inner.lock().unwrap();
            match inner.session.clone() {
                Some(session) => Some(session),
                None if inner.process.is_some() => {
                    inner.pending_messages.push_back(message.clone());
                    None
                }
                None => return Err("Codex App Server is not initialized".to_string()),
            }
        };
        if let Some(session) = session {
            self.spawn_turn(session, message);
        }
        Ok(())
    }

    /// Clear a dead process/session so a later `start()` (resync) doesn't see
    /// its own stale `process.is_some()` guard and wrongly no-op instead of
    /// respawning. ReAgent P1 on PR #3215's third review: `start()`'s own
    /// initialize/thread-setup failure branches set STATUS_DONE and shut the
    /// process down without this, unlike the analogous EOF path in
    /// `spawn_event_loop` — a handshake failure permanently bricked the block
    /// with no recovery short of replacing the controller entirely.
    fn clear_process_and_session(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.process = None;
        inner.session = None;
    }

    fn stop_process(&self) {
        let process = self.inner.lock().unwrap().process.clone();
        let weak = self.self_ref.lock().unwrap().clone();
        if let Some(process) = process {
            tokio::spawn(async move {
                let exit = process.shutdown(Duration::from_secs(2)).await;
                // Clear process/session immediately instead of waiting for the
                // event loop to separately observe the resulting EOF — without
                // this, get_runtime_status/send_message report a live process
                // for up to the 2s shutdown grace period (or longer) after a
                // caller-initiated stop already asked it to go away.
                let exit_code = match exit {
                    Ok(AppServerExit::Clean { code, .. })
                    | Ok(AppServerExit::BeforeInitialize { code, .. })
                    | Ok(AppServerExit::NonZero { code, .. }) => code.unwrap_or(0),
                    _ => 0,
                };
                if let Some(controller) = weak.and_then(|weak| Weak::upgrade(&weak)) {
                    controller.health_monitor.set_exited(exit_code);
                    let mut inner = controller.inner.lock().unwrap();
                    inner.process = None;
                    inner.session = None;
                    inner.proc_exit_code = exit_code;
                }
            });
        }
    }
}

impl Controller for AppServerController {
    fn start(
        &self,
        block_meta: crate::backend::obj::MetaMapType,
        _rt_opts: Option<serde_json::Value>,
        _force: bool,
    ) -> Result<(), String> {
        if self.inner.lock().unwrap().process.is_some() {
            return Ok(());
        }
        let command = self.command_from_meta(&block_meta)?;
        let process = Arc::new(
            AppServerProcess::spawn(command, AppServerLimits::default())
                .map_err(|error| error.to_string())?,
        );
        let process_for_task = process.clone();
        let weak = self.self_ref.lock().unwrap().clone();
        self.inner.lock().unwrap().process = Some(process.clone());
        self.set_status(STATUS_RUNNING);
        tokio::spawn(async move {
            let Some(controller) = weak.and_then(|weak| Weak::upgrade(&weak)) else {
                return;
            };
            if let Err(error) = process_for_task
                .transport
                .initialize(INITIALIZE_TIMEOUT)
                .await
            {
                tracing::warn!(block_id = %controller.block_id, error = %error, "Codex App Server initialize failed");
                controller.set_status(STATUS_DONE);
                controller.clear_process_and_session();
                let _ = process_for_task.shutdown(Duration::from_secs(1)).await;
                return;
            }
            let transport = process_for_task.transport.clone();
            let session = Arc::new(CodexAppServerSession::new(transport));
            let cwd = crate::backend::obj::meta_get_string(&block_meta, "cmd:cwd", "");
            let model = crate::backend::obj::meta_get_string(&block_meta, "agent:model", "");
            let stored_thread =
                crate::backend::obj::meta_get_string(&block_meta, "agent:sessionid", "");
            let thread = if stored_thread.is_empty() {
                session
                    .start_thread(
                        ThreadStartOptions {
                            model: (!model.is_empty()).then_some(model),
                            cwd: (!cwd.is_empty()).then_some(cwd),
                            ..Default::default()
                        },
                        OPERATION_TIMEOUT,
                    )
                    .await
            } else {
                session
                    .resume_thread(
                        stored_thread,
                        (!model.is_empty()).then_some(model),
                        (!cwd.is_empty()).then_some(cwd),
                        OPERATION_TIMEOUT,
                    )
                    .await
            };
            let thread_id = match thread {
                Ok(thread_id) => thread_id,
                Err(error) => {
                    tracing::warn!(block_id = %controller.block_id, error = %error, "Codex App Server thread setup failed");
                    controller.set_status(STATUS_DONE);
                    controller.clear_process_and_session();
                    let _ = process_for_task.shutdown(Duration::from_secs(1)).await;
                    return;
                }
            };
            if let (Some(store), Some(event_bus)) = (&controller.wstore, &controller.event_bus) {
                core::persist_session_id(
                    &controller.block_id,
                    &thread_id,
                    &Some(store.clone()),
                    &Some(event_bus.clone()),
                );
            }
            controller.inner.lock().unwrap().session = Some(session.clone());
            controller.set_status(STATUS_RUNNING);
            // Only one turn can be active at a time — start just the first
            // queued message now; the rest stay queued and are sent one at a
            // time as each turn completes (spawn_event_loop's `TurnCompleted`
            // handler). Draining and spawning all of them here concurrently
            // meant every message but the first immediately hit
            // `TurnAlreadyActive` and was silently lost (ReAgent P1, PR #3215).
            let first_pending_message = controller.inner.lock().unwrap().pending_messages.pop_front();
            if let Some(first_pending_message) = first_pending_message {
                controller.spawn_turn(session.clone(), first_pending_message);
            }
            controller.spawn_event_loop(process_for_task, session);
        });
        Ok(())
    }

    fn stop(&self, _graceful: bool, new_status: &str) -> Result<(), String> {
        self.stop_process();
        self.set_status(new_status);
        Ok(())
    }

    fn get_runtime_status(&self) -> BlockControllerRuntimeStatus {
        let inner = self.inner.lock().unwrap();
        BlockControllerRuntimeStatus {
            blockid: self.block_id.clone(),
            version: inner.status_version,
            shellprocstatus: inner.proc_status.clone(),
            shellprocconnname: "local".to_string(),
            shellprocexitcode: inner.proc_exit_code,
            spawn_ts_ms: None,
            is_agent_pane: true,
            turn_active: self.health_monitor.is_active_turn(),
        }
    }

    fn send_input(&self, input: BlockInputUnion, _seq: Option<u64>) -> Result<(), String> {
        if let Some(signal) = input.sig_name.as_deref() {
            if signal == "SIGINT" || signal == "SIGTERM" {
                self.stop_process();
                return Ok(());
            }
            return Err(format!("unsupported App Server signal {signal}"));
        }
        if let Some(data) = input.input_data {
            let message =
                String::from_utf8(data).map_err(|error| format!("input is not UTF-8: {error}"))?;
            return self.send_message(message);
        }
        Ok(())
    }

    fn controller_type(&self) -> &str {
        super::BLOCK_CONTROLLER_APP_SERVER
    }

    fn block_id(&self) -> &str {
        &self.block_id
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::obj::MetaMapType;

    fn controller() -> AppServerController {
        AppServerController::new(
            "tab-1".to_string(),
            "block-1".to_string(),
            None,
            None,
            None,
            None,
        )
    }

    #[test]
    fn starts_idle_and_reports_the_app_server_controller_type() {
        let controller = controller();
        let status = controller.get_runtime_status();
        assert_eq!(status.shellprocstatus, STATUS_INIT);
        assert_eq!(status.blockid, "block-1");
        assert!(status.is_agent_pane);
        assert_eq!(
            controller.controller_type(),
            super::super::BLOCK_CONTROLLER_APP_SERVER
        );
    }

    #[test]
    fn rejects_empty_app_server_arguments_before_spawning() {
        let controller = controller();
        let mut meta = MetaMapType::new();
        meta.insert("cmd".to_string(), serde_json::json!("codex"));
        meta.insert("cmd:args".to_string(), serde_json::json!([]));
        let error = controller.command_from_meta(&meta).unwrap_err();
        assert!(error.contains("must include app-server arguments"));
    }

    #[tokio::test]
    async fn input_requires_a_completed_handshake() {
        let controller = controller();
        let error = controller
            .send_input(BlockInputUnion::data(b"hello".to_vec()), None)
            .unwrap_err();
        assert!(error.contains("not initialized"));
    }

    // ── Regression coverage for ReAgent's PR #3215 review ────────────────────
    // Four bugs: (1) `spawn_turn`'s error path couldn't tell "a second turn
    // was rejected because one is already running" from "the process is
    // actually dead", (2) `spawn_event_loop`'s EOF branch never reaped the
    // child or cleared stale process/session state, (3) `deliver_agent_message`
    // had no route to this controller at all (covered in blockcontroller/mod.rs's
    // own tests), (4) `agent_open.rs`'s cli_args selection never looked at
    // `provider.app_server` (covered in agent_open.rs's own tests). Plus one
    // deadlock this investigation found independently: `set_status` re-locks
    // its own mutex from inside the guard it's still holding, masked in every
    // pre-existing test here because they all use `broker: None`.

    fn controller_with_broker() -> (Arc<AppServerController>, Arc<wps::Broker>) {
        let broker = Arc::new(wps::Broker::new());
        let controller = Arc::new(AppServerController::new(
            "tab-1".to_string(),
            "block-1".to_string(),
            Some(broker.clone()),
            None,
            None,
            None,
        ));
        controller.set_self_ref();
        (controller, broker)
    }

    #[test]
    fn set_status_with_a_broker_does_not_deadlock_locking_its_own_mutex() {
        let (controller, _broker) = controller_with_broker();
        let for_thread = controller.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for_thread.stop(false, STATUS_DONE).unwrap();
            let _ = tx.send(());
        });
        rx.recv_timeout(Duration::from_secs(2)).expect(
            "set_status must not deadlock re-locking its own mutex once a broker is configured \
             — every production controller has one",
        );
        assert_eq!(controller.get_runtime_status().shellprocstatus, STATUS_DONE);
    }

    fn fake_server_binary() -> &'static std::path::PathBuf {
        static BINARY: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
        BINARY.get_or_init(|| {
            let temp = tempfile::tempdir().expect("create fake server build dir");
            let mut binary = temp.path().join("fake-app-server-ctrl");
            if cfg!(windows) {
                binary.set_extension("exe");
            }
            let source = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests")
                .join("fixtures")
                .join("fake_app_server.rs");
            let output = std::process::Command::new("rustc")
                .arg("--edition=2021")
                .arg(source)
                .arg("-o")
                .arg(&binary)
                .output()
                .expect("run rustc for fake App Server");
            assert!(
                output.status.success(),
                "fake App Server compilation failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            std::mem::forget(temp);
            binary
        })
    }

    fn app_server_meta(mode: &str) -> MetaMapType {
        let mut meta = MetaMapType::new();
        meta.insert(
            "cmd".to_string(),
            serde_json::json!(fake_server_binary().to_string_lossy()),
        );
        meta.insert(
            "cmd:env".to_string(),
            serde_json::json!({"AGENTMUX_FAKE_APP_SERVER_MODE": mode}),
        );
        meta
    }

    async fn wait_until(predicate: impl Fn() -> bool, timeout: Duration) -> bool {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if predicate() {
                return true;
            }
            if tokio::time::Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    /// The session's own `turn_id`, straight from `CodexAppServerSession`'s
    /// state — unlike `health_monitor.is_active_turn()` (which flips to
    /// `true` synchronously in `spawn_turn`, BEFORE the `turn/start` request
    /// even goes out), this only becomes `Some` once the round-trip actually
    /// completes. Tests that need to guarantee a SECOND `send_message` hits
    /// `start_turn`'s local `TurnAlreadyActive` check (rather than racing it
    /// and sending its own concurrent `turn/start`) must synchronize on this,
    /// not on `is_active_turn()`.
    fn session_turn_id(controller: &AppServerController) -> Option<String> {
        controller
            .inner
            .lock()
            .unwrap()
            .session
            .as_ref()
            .and_then(|session| session.snapshot().turn_id)
    }

    #[tokio::test]
    async fn spawn_event_loop_reaps_the_process_and_clears_stale_state_on_exit() {
        let (controller, _broker) = controller_with_broker();
        controller
            .start(app_server_meta("exit-after-thread-start"), None, false)
            .unwrap();

        let reached_done = wait_until(
            || controller.get_runtime_status().shellprocstatus == STATUS_DONE,
            Duration::from_secs(5),
        )
        .await;
        assert!(
            reached_done,
            "controller never reached STATUS_DONE after the fake server exited"
        );

        let status = controller.get_runtime_status();
        assert_eq!(
            status.shellprocexitcode, 42,
            "must report the process's real exit code instead of a hardcoded stale one"
        );
        assert!(!status.turn_active);

        let error = controller
            .send_message("too late".to_string())
            .unwrap_err();
        assert!(
            error.contains("not initialized"),
            "process/session must be cleared on exit so a post-exit message is rejected \
             instead of silently accepted by a session bound to a dead transport, got {error:?}",
        );
    }

    #[tokio::test]
    async fn a_second_message_while_a_turn_is_active_does_not_kill_the_controller() {
        let (controller, _broker) = controller_with_broker();
        controller
            .start(app_server_meta("ready-for-turn"), None, false)
            .unwrap();

        // `start()` only completes the handshake/thread setup — it does not
        // start a turn on its own. Wait for the session to actually exist
        // before sending the first message.
        let session_ready = wait_until(
            || controller.inner.lock().unwrap().session.is_some(),
            Duration::from_secs(5),
        )
        .await;
        assert!(session_ready, "controller session was never established");

        controller
            .send_message("first message".to_string())
            .unwrap();

        // Wait for the ACTUAL turn/start round-trip to finish (not just
        // `is_active_turn()`, which flips true before the request is even
        // sent) so the second message below is guaranteed to hit the local
        // `TurnAlreadyActive` check instead of racing it.
        let turn_id_set =
            wait_until(|| session_turn_id(&controller).is_some(), Duration::from_secs(5)).await;
        assert!(turn_id_set, "first turn's turn/start round-trip never completed");
        assert!(controller.health_monitor.is_active_turn());
        assert_eq!(controller.get_runtime_status().shellprocstatus, STATUS_RUNNING);

        // `start_turn`'s `TurnAlreadyActive` check is local (state.turn_id is
        // already Some from the first turn) — this never touches the network,
        // so no fixture response is needed for it to fire.
        controller
            .send_message("second message while the first turn is still running".to_string())
            .unwrap();

        // Give the rejected turn's spawned task a moment to run (and, pre-fix,
        // wrongly tear down the controller out from under the live first turn).
        tokio::time::sleep(Duration::from_millis(200)).await;

        assert!(
            controller.health_monitor.is_active_turn(),
            "the ORIGINAL turn is still active; a rejected second turn must not clear that"
        );
        assert_eq!(
            controller.get_runtime_status().shellprocstatus,
            STATUS_RUNNING,
            "a rejected second turn must not mark a live controller done"
        );
        assert_eq!(
            controller.inner.lock().unwrap().pending_messages.len(),
            1,
            "the rejected message must be queued, not dropped (ReAgent P1, PR #3215 2nd review)"
        );

        // Cleanup: this fixture mode idles forever rather than exiting on its
        // own, and the event loop task holds its own Arc<AppServerProcess> for
        // as long as it's reading — `kill_on_drop` never fires without an
        // explicit stop, which would otherwise leak the child for the rest of
        // this test binary's run.
        controller.stop(true, STATUS_DONE).unwrap();
    }

    /// End-to-end version of the test above: the queued message must actually
    /// get SENT once the turn it collided with completes, not just survive in
    /// the queue forever. ReAgent P1 on PR #3215's second review.
    #[tokio::test]
    async fn a_queued_message_is_sent_once_the_active_turn_completes() {
        let (controller, _broker) = controller_with_broker();
        controller
            .start(app_server_meta("requeue-after-turn-completes"), None, false)
            .unwrap();

        let session_ready = wait_until(
            || controller.inner.lock().unwrap().session.is_some(),
            Duration::from_secs(5),
        )
        .await;
        assert!(session_ready, "controller session was never established");

        controller
            .send_message("first message".to_string())
            .unwrap();
        let turn_id_set =
            wait_until(|| session_turn_id(&controller).is_some(), Duration::from_secs(5)).await;
        assert!(turn_id_set, "first turn's turn/start round-trip never completed");

        // The fixture waits 300ms after responding to turn/start before
        // sending turn/completed specifically so this has time to land while
        // state.turn_id is still Some — i.e. the rejection-and-queue path,
        // not a normal second turn started after the first already finished.
        controller
            .send_message("queued while turn 1 is active".to_string())
            .unwrap();
        let queued = wait_until(
            || controller.inner.lock().unwrap().pending_messages.len() == 1,
            Duration::from_secs(1),
        )
        .await;
        assert!(queued, "second message was not queued — test is racing the fixture's notification");

        // Once turn/completed arrives, the event loop must drain the queue
        // and start the second turn — the fixture only responds to a SECOND
        // turn/start if the controller actually sends one.
        let requeued_message_was_sent = wait_until(
            || controller.inner.lock().unwrap().pending_messages.is_empty(),
            Duration::from_secs(5),
        )
        .await;
        assert!(
            requeued_message_was_sent,
            "queued message was never drained after the active turn completed"
        );
        let second_turn_active = wait_until(
            || controller.health_monitor.is_active_turn(),
            Duration::from_secs(5),
        )
        .await;
        assert!(
            second_turn_active,
            "the requeued message's turn never became active — it was drained but never actually sent"
        );

        controller.stop(true, STATUS_DONE).unwrap();
    }

    /// ReAgent P1 on PR #3215's third review: a handshake failure (here,
    /// the child dying before `initialize` completes) must not permanently
    /// brick the block. `start()`'s own guard (`process.is_some()`) means a
    /// stale process/session left behind by a failed handshake makes every
    /// later resync a silent no-op instead of a real respawn.
    #[tokio::test]
    async fn a_failed_handshake_clears_state_so_a_later_start_can_actually_respawn() {
        let (controller, _broker) = controller_with_broker();
        controller
            .start(app_server_meta("exit-before-init"), None, false)
            .unwrap();

        let reached_done = wait_until(
            || controller.get_runtime_status().shellprocstatus == STATUS_DONE,
            Duration::from_secs(5),
        )
        .await;
        assert!(reached_done, "controller never reached STATUS_DONE after the failed handshake");
        assert!(
            controller.inner.lock().unwrap().process.is_none(),
            "a failed handshake must clear the dead process, not leave start()'s guard permanently tripped"
        );
        assert!(controller.inner.lock().unwrap().session.is_none());

        // If the bug were still present, this would silently no-op (the old
        // `process.is_some()` guard still passing) instead of actually
        // spawning a new child — and the assertions below would time out.
        controller
            .start(app_server_meta("ready-for-turn"), None, false)
            .unwrap();
        let respawned_session_ready = wait_until(
            || controller.inner.lock().unwrap().session.is_some(),
            Duration::from_secs(5),
        )
        .await;
        assert!(
            respawned_session_ready,
            "start() after a failed handshake never respawned — the block is permanently bricked"
        );

        controller.stop(true, STATUS_DONE).unwrap();
    }
}
