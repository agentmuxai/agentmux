// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! First-class controller shell for a Codex App Server process.
//!
//! The protocol implementation lives in [`super::app_server`] and
//! [`super::app_server_protocol`]. This module owns the AgentMux controller
//! lifecycle: command construction, process/thread startup, turn dispatch,
//! block output publication, and shutdown. Codex remains opt-in until the
//! provider registry flips its controller type.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use tokio::process::Command;

use super::app_server::{AppServerIncoming, AppServerLimits, AppServerProcess};
use super::app_server_protocol::{CodexAppServerEvent, CodexAppServerSession, ThreadStartOptions};
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
        self.inner.lock().unwrap().proc_status = status.to_string();
        self.inner.lock().unwrap().status_version += 1;
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
            if let Some(controller) = weak.as_ref().and_then(Weak::upgrade) {
                controller.health_monitor.set_exited(1);
                controller.set_status(STATUS_DONE);
            }
        });
    }

    pub fn send_message(&self, message: String) -> Result<(), String> {
        let session = self
            .inner
            .lock()
            .unwrap()
            .session
            .clone()
            .ok_or_else(|| "Codex App Server is not initialized".to_string())?;
        let health = self.health_monitor.clone();
        let weak = self.self_ref.lock().unwrap().clone();
        health.set_active_turn(true);
        self.set_status(STATUS_RUNNING);
        tokio::spawn(async move {
            if let Err(error) = session.start_turn(message, OPERATION_TIMEOUT).await {
                health.set_active_turn(false);
                if let Some(controller) = weak.and_then(|weak| Weak::upgrade(&weak)) {
                    controller.set_status(STATUS_DONE);
                    tracing::warn!(block_id = %controller.block_id, error = %error, "Codex App Server turn failed to start");
                }
            }
        });
        Ok(())
    }

    fn stop_process(&self) {
        let process = self.inner.lock().unwrap().process.clone();
        if let Some(process) = process {
            tokio::spawn(async move {
                let _ = process.shutdown(Duration::from_secs(2)).await;
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
}
