// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The steps of [`resync_controller`], one function each: pick the effective controller type,
//! decide what to do with a controller the block already has, then build and start a new one of
//! the right type. Moved out of `resync_controller` without a change in behaviour.

use super::*;

/// Everything `resync_controller` was called with, gathered so the steps share one value
/// instead of fifteen parameters.
pub(super) struct ResyncCtx<'a> {
    pub block: &'a Block,
    pub tab_id: &'a str,
    pub rt_opts: Option<serde_json::Value>,
    pub force: bool,
    pub respawn_if_done: bool,
    pub broker: Option<Arc<Broker>>,
    pub event_bus: Option<Arc<EventBus>>,
    pub mstore: Option<Arc<Store>>,
    pub filestore: Option<Arc<FileStore>>,
    pub id_store: Option<Arc<Store>>,
    pub identity_store: Option<Arc<Store>>,
    pub registry: Option<Arc<crate::registry::Registry>>,
    pub boot_id: Arc<str>,
    pub auth_key: &'a str,
    pub config: Option<Arc<crate::backend::wconfig::ConfigState>>,
}

/// Container agents (agentMode == "container") require a subprocess controller for
/// per-turn docker exec. Persistent is incompatible. Override any stale "persistent"
/// value that may have been written before this invariant was enforced at creation time.
pub(super) fn effective_controller_type(block: &Block, controller_type: String) -> String {
    let runtime = super::super::runtime_adapter::RuntimeKind::of_block_meta(&block.meta).adapter();
    if !runtime.accepts_persistent_controller() && controller_type == BLOCK_CONTROLLER_PERSISTENT {
        tracing::warn!(
            block_id = %block.oid,
            "container agent has persistent controller in meta — overriding to subprocess"
        );
        BLOCK_CONTROLLER_SUBPROCESS.to_string()
    } else {
        controller_type
    }
}

impl ResyncCtx<'_> {
    /// Decide what to do with the controller the block already has. `Some(result)` ends the
    /// resync with that result; `None` means the old controller was stopped and removed, so a
    /// new one is to be created.
    pub(super) fn resync_existing(
        &self,
        ctrl: &Arc<dyn Controller>,
        controller_type: &str,
    ) -> Option<Result<(), String>> {
        let block_id = &self.block.oid;
        let block_meta = &self.block.meta;
        let needs_replace = if ctrl.controller_type() != controller_type || self.force {
            true // Type changed or forced restart
        } else {
            let status = ctrl.get_runtime_status();
            // Check if connection changed
            let new_conn = super::super::obj::meta_get_string(block_meta, META_KEY_CONNECTION, "local");
            status.shellprocconnname != new_conn
        };

        // A forced resync that lands mid-turn on a persistent agent must NOT
        // tear the controller down: doing so kills the CLI process with the
        // user's in-flight message already written to its stdin, and nothing
        // replays it — `stop_process` records a `StopRequested` (which the
        // resume machine reads as an explicit user stop and so suppresses the
        // retry), the old controller's queue is discarded with it, and the
        // replacement "spawns on first message", i.e. does nothing. The turn
        // vanished with no response and no error. Diagnosed live on AgentX,
        // 2026-08-28.
        //
        // Instead the controller restarts itself the moment the turn ends,
        // picking up the new `cmd:args` (rebuilt from block meta at spawn) on
        // the next message. Nothing is lost by waiting: model/effort flags are
        // baked in at spawn, so they could never have applied to the turn
        // already running.
        //
        // Scoped to a runtime-config tweak: the controller must be persistent
        // now AND still be persistent afterwards. A controller-TYPE change is a
        // genuine replacement that has to happen immediately, even mid-turn.
        //
        // Checking only the EXISTING type was a real bug: the container-agent
        // override (`effective_controller_type`) rewrites `controller_type`
        // persistent -> subprocess, so a forced resync landing mid-turn on such
        // a block would have deferred and silently kept the persistent
        // controller — the exact incompatibility that override exists to
        // correct, and the opposite of what this comment claims.
        if needs_replace
            && is_runtime_config_only_replace(ctrl.controller_type(), controller_type, self.force)
        {
            if let Some(p) = ctrl
                .as_any()
                .downcast_ref::<persistent::PersistentSubprocessController>()
            {
                if p.request_restart_when_idle() {
                    return Some(Ok(()));
                }
            }
        }

        if needs_replace {
            // stop_for_replace + remove_controller_entry_only, NOT stop +
            // delete_controller: a session restart/resync must not tear
            // down the block's shared process tracker — only the old
            // controller's own CLI process should die. See
            // docs/specs/SPEC_BACKGROUND_TASK_TEARDOWN_SURVIVAL_2026_08_20.md.
            let _ = crate::sagas::agent_teardown::replace_now(ctrl.as_ref());
            remove_controller_entry_only(block_id);
            return None;
        }

        // Existing controller is fine, just check if it needs starting
        let status = ctrl.get_runtime_status();
        tracing::info!(
            block_id = %block_id,
            status = %status.shellprocstatus,
            "[dnd-debug] existing controller — skipping spawn (no cmd:cwd seed)"
        );
        if status.shellprocstatus == STATUS_INIT {
            return Some(self.restart_existing(ctrl));
        }
        if status.shellprocstatus == STATUS_DONE {
            if self.respawn_if_done {
                return Some(self.restart_existing(ctrl));
            }
            return Some(Err(RESYNC_ERR_ALREADY_EXITED.to_string()));
        }
        Some(Ok(()))
    }

    fn restart_existing(&self, ctrl: &Arc<dyn Controller>) -> Result<(), String> {
        refresh_agent_skill_files(&self.block.meta, &self.mstore, &self.identity_store);
        ctrl.start(self.block.meta.clone(), self.rt_opts.clone(), self.force)
    }

    /// Create a controller of `controller_type`, register it and start it.
    pub(super) fn create_and_start(self, controller_type: &str) -> Result<(), String> {
        refresh_agent_skill_files(&self.block.meta, &self.mstore, &self.identity_store);
        match controller_type {
            // A durable SSH pane: the shell lives in the helper's session on the
            // host, reached over ssh (durable_ssh.rs). Same `shell` type and
            // connection name as the meta says, so a resync keeps it.
            BLOCK_CONTROLLER_SHELL
                if durable_mode::wants(
                    &self.block.meta,
                    self.config.as_ref().map(|c| c.get_full_config()).as_deref(),
                ) =>
            {
                self.start_durable_ssh()
            }
            BLOCK_CONTROLLER_SHELL | BLOCK_CONTROLLER_CMD => self.start_shell(controller_type),
            BLOCK_CONTROLLER_SUBPROCESS => self.start_subprocess(),
            BLOCK_CONTROLLER_PERSISTENT => self.start_persistent(),
            BLOCK_CONTROLLER_ACP => self.start_acp(),
            BLOCK_CONTROLLER_MSP => self.start_msp(),
            BLOCK_CONTROLLER_APP_SERVER => self.start_app_server(),
            _ => Err(format!("unknown controller type: {controller_type}")),
        }
    }

    /// When the user declines the helper, the pane restarts as a plain
    /// SSH terminal: the same resync, forced, which then finds the
    /// pane no longer durable (durable_mode::wants).
    fn durable_fallback(&self) -> durable_ssh::Fallback {
        let block_id = self.block.oid.clone();
        let (tab_id, rt_opts) = (self.tab_id.to_string(), self.rt_opts.clone());
        let (broker, event_bus, mstore, filestore) = (
            self.broker.clone(),
            self.event_bus.clone(),
            self.mstore.clone(),
            self.filestore.clone(),
        );
        let (id_store, identity_store, registry) =
            (self.id_store.clone(), self.identity_store.clone(), self.registry.clone());
        let (boot_id, auth_key, config) = (self.boot_id.clone(), self.auth_key.to_string(), self.config.clone());
        Arc::new(move || {
            let Some(block) = mstore.as_ref().and_then(|s| s.get::<Block>(&block_id).ok().flatten()) else {
                return false;
            };
            // Still durable: a restart would only start the same
            // durable controller again. Leave the pane stopped.
            if durable_mode::wants(&block.meta, config.as_ref().map(|c| c.get_full_config()).as_deref()) {
                tracing::warn!(block_id = %block_id, "durable ssh: still durable, not falling back");
                return false;
            }
            if let Err(e) = resync_controller(
                &block,
                &tab_id,
                rt_opts.clone(),
                true,
                true,
                broker.clone(),
                event_bus.clone(),
                mstore.clone(),
                filestore.clone(),
                id_store.clone(),
                identity_store.clone(),
                registry.clone(),
                boot_id.clone(),
                &auth_key,
                config.clone(),
            ) {
                tracing::warn!(block_id = %block_id, error = %e, "durable ssh: plain terminal fallback failed");
                return false;
            }
            true
        })
    }

    fn start_durable_ssh(self) -> Result<(), String> {
        let fallback = self.durable_fallback();
        let ctrl = Arc::new(durable_ssh::DurableSshController::new(
            self.block.oid.clone(),
            self.broker,
            self.event_bus,
            self.mstore,
            self.filestore,
            self.auth_key.to_string(),
            self.config.clone(),
            Some(fallback),
        ));
        register_controller(&self.block.oid, ctrl.clone());
        ctrl.start(self.block.meta.clone(), self.rt_opts, self.force)
    }

    fn start_shell(self, controller_type: &str) -> Result<(), String> {
        let ctrl = shell::ShellController::new(
            controller_type.to_string(),
            self.tab_id.to_string(),
            self.block.oid.clone(),
            self.broker,
            self.event_bus,
            self.mstore,
            self.filestore,
            self.auth_key.to_string(),
        )
        .with_config(self.config);
        let ctrl = Arc::new(ctrl);
        register_controller(&self.block.oid, ctrl.clone());
        let result = ctrl.start(self.block.meta.clone(), self.rt_opts, self.force);
        // ShellController::new starts with is_agent_pane: false and start()
        // only sets it (synchronously, before returning — shell/lifecycle.rs's
        // start()) after actually spawning. register_controller's notification
        // above therefore fired before list_agent_panes()'s is_agent() filter
        // would have included this block; notify again now that start() has
        // run so the classified state gets an authoritative announcement too,
        // instead of depending solely on a proxy event or the poll.
        notify_tracked_blocks_changed();
        result
    }

    fn start_subprocess(self) -> Result<(), String> {
        let ctrl = subprocess::SubprocessController::new(
            self.tab_id.to_string(),
            self.block.oid.clone(),
            self.broker,
            self.event_bus,
            self.mstore,
            self.filestore,
            self.registry,
            self.boot_id,
        );
        let ctrl = Arc::new(ctrl);
        ctrl.set_self_ref();
        register_controller(&self.block.oid, ctrl.clone());
        let result = ctrl.start(self.block.meta.clone(), self.rt_opts, self.force);
        notify_tracked_blocks_changed();
        result
    }

    fn start_persistent(self) -> Result<(), String> {
        let ctrl = persistent::PersistentSubprocessController::new(
            self.tab_id.to_string(),
            self.block.oid.clone(),
            self.broker,
            self.event_bus,
            self.mstore,
            self.filestore,
        )
        .with_identity_stores(self.id_store, self.identity_store, self.auth_key.to_string())
        // One live instance per agent (SPEC_AGENT_SINGLE_LIVE_INSTANCE_2026_09_24).
        .with_agent_lease_store(self.registry, self.boot_id);
        let ctrl = Arc::new(ctrl);
        ctrl.set_self_ref();
        register_controller(&self.block.oid, ctrl.clone());
        let result = ctrl.start(self.block.meta.clone(), self.rt_opts, self.force);
        notify_tracked_blocks_changed();
        result
    }

    fn start_acp(self) -> Result<(), String> {
        let ctrl = acp::AcpController::new(
            self.tab_id.to_string(),
            self.block.oid.clone(),
            self.broker,
            self.event_bus,
            self.mstore,
            self.filestore,
        )
        .with_auth_key(self.auth_key.to_string());
        let ctrl = Arc::new(ctrl);
        register_controller(&self.block.oid, ctrl.clone());
        let result = ctrl.start(self.block.meta.clone(), self.rt_opts, self.force);
        notify_tracked_blocks_changed();
        result
    }

    fn start_msp(self) -> Result<(), String> {
        let ctrl = msp::MspController::new(
            self.block.oid.clone(),
            self.broker,
            self.event_bus,
            self.mstore,
            self.filestore,
        )
        .with_auth_key(self.auth_key.to_string());
        let ctrl = Arc::new(ctrl);
        register_controller(&self.block.oid, ctrl.clone());
        let result = ctrl.start(self.block.meta.clone(), self.rt_opts, self.force);
        notify_tracked_blocks_changed();
        result
    }

    fn start_app_server(self) -> Result<(), String> {
        let ctrl = app_server_controller::AppServerController::new(
            self.block.oid.clone(),
            self.broker,
            self.event_bus,
            self.mstore,
            self.filestore,
        );
        let ctrl = Arc::new(ctrl);
        ctrl.set_self_ref();
        register_controller(&self.block.oid, ctrl.clone());
        ctrl.start(self.block.meta.clone(), self.rt_opts, self.force)
    }
}
