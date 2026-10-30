// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//! Getting a container agent's container ready for a turn: the container is
//! running, and the provider CLI is installed in it.
//!
//! This is the first thing a container turn does, inside the turn's own task
//! (`SubprocessController::spawn_container_turn`), NOT inside the RPC handler
//! that accepted the message: downloading the image takes minutes, and the RPC
//! engine drops a handler after five seconds, which cancelled the download for
//! good. The handler only records the [`ContainerStartup`] on the controller
//! and queues the turn.
//!
//! Spec: docs/specs/SPEC_CONTAINER_AGENTS_AT_PARITY_2026_10_27.md (P0, P1, P2),
//! docs/specs/SPEC_CONTAINER_AGENTS_WORK_FOR_EVERYONE_2026_10_07.md.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use serde_json::{json, Value};

use crate::backend::cli_notice::{self, ContainerStartState, InstallState};
use crate::backend::container::{CliProvision, ContainerError, ContainerManager, ContainerMountSpec, StartEvent};
use crate::backend::container_cli::CliInstall;
use crate::backend::container_image::resolve_container_image;

/// Everything needed to get a container agent's container ready, owned, so it
/// can outlive the request that asked for it.
pub(crate) struct ContainerStartup {
    pub cm: ContainerManager,
    pub block_id: String,
    pub container_name: String,
    /// The stored `agent:container_image`; empty means the default image.
    pub image: String,
    pub volumes: Vec<String>,
    pub mount_spec: ContainerMountSpec,
    pub provider_id: String,
    pub container_command: String,
    pub broker: Arc<crate::backend::mps::Broker>,
    pub filestore: Arc<crate::backend::storage::filestore::FileStore>,
    pub mstore: Arc<crate::backend::storage::store::Store>,
}

/// The step the controller runs at the start of each container turn.
pub(crate) fn startup_step(startup: ContainerStartup) -> crate::backend::blockcontroller::subprocess::StartupStep {
    let startup = Arc::new(startup);
    Arc::new(move || {
        let startup = Arc::clone(&startup);
        Box::pin(async move { startup.run().await })
    })
}

impl ContainerStartup {
    /// For the container agent in `block_meta`: its container, and the mounts for the account bound to it
    /// (`env_vars` as the identity injection left them) and its workspace. `AGENTMUX_LOCAL_URL` in
    /// `env_vars` is rewritten to how the container reaches this srv. The turn's start-up and the
    /// pre-start both build theirs here, so the container is the same whichever comes first.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn for_block(
        cm: ContainerManager,
        block_id: &str,
        block_meta: &crate::backend::obj::MetaMapType,
        env_vars: &mut std::collections::HashMap<String, String>,
        working_dir: &str,
        broker: Arc<crate::backend::mps::Broker>,
        filestore: Arc<crate::backend::storage::filestore::FileStore>,
        mstore: Arc<crate::backend::storage::store::Store>,
    ) -> Self {
        let mut spec = ContainerSpec::of(block_meta, env_vars, working_dir);
        spec.mount_spec.limits = crate::backend::container::limits::ContainerLimits::of_agent(&mstore, &crate::backend::obj::meta_get_string(block_meta, "agentId", ""));
        Self {
            cm,
            block_id: block_id.to_string(),
            container_name: spec.container_name,
            image: spec.image,
            volumes: spec.volumes,
            mount_spec: spec.mount_spec,
            provider_id: spec.provider_id,
            container_command: spec.container_command,
            broker,
            filestore,
            mstore,
        }
    }
}

/// What a container agent's pane says about its container, apart from the services that run it.
struct ContainerSpec {
    container_name: String,
    image: String,
    volumes: Vec<String>,
    mount_spec: ContainerMountSpec,
    provider_id: String,
    container_command: String,
}

impl ContainerSpec {
    fn of(block_meta: &crate::backend::obj::MetaMapType, env_vars: &mut std::collections::HashMap<String, String>, working_dir: &str) -> Self {
        use crate::backend::obj::meta_get_string;
        // The agent id (a UUID) is always valid as a Docker name; a display name can have spaces.
        let agent_id = meta_get_string(block_meta, "agentId", "");
        let volumes_json = meta_get_string(block_meta, "agent:container_volumes", "[]");
        // `CLAUDE_CONFIG_DIR` is a host path, stripped before exec by the env denylist, so mounting it is the
        // only way the in-container CLI sees the bound account's credentials.
        let mount_spec = ContainerMountSpec::for_agent(env_vars, working_dir);
        // A container can't reach the loopback address this process sees.
        if let Ok(local_url) = std::env::var("AGENTMUX_LOCAL_URL") {
            env_vars.insert("AGENTMUX_LOCAL_URL".to_string(), crate::backend::container::rewrite_local_url_for_container(&local_url));
        }
        Self {
            container_name: crate::backend::container::container_name_for_slug(&agent_id),
            image: meta_get_string(block_meta, "agent:container_image", ""),
            volumes: serde_json::from_str(&volumes_json).unwrap_or_default(),
            mount_spec,
            provider_id: meta_get_string(block_meta, "agentProvider", "claude"),
            container_command: meta_get_string(block_meta, "agent:container_command", "claude"),
        }
    }
}

/// Whether a pre-start applies: a container agent whose pane has run nothing yet. A pane that is running a
/// turn, or has run one, is left to the turn's own start-up.
fn prestart_applies(agent_mode: &str, runtime_status: &str) -> bool {
    agent_mode == "container" && runtime_status == crate::backend::blockcontroller::STATUS_INIT
}

/// Where a block's pre-start is.
enum Prestart {
    /// Preparing; no task yet.
    Preparing,
    /// The pane closed before the task existed: it must not start one.
    Cancelled,
    /// Running; closing the pane aborts it.
    Running(tokio::task::AbortHandle),
}

/// Blocks whose pre-start is under way, so a second request for one does nothing, and how to end it.
static PRESTARTING: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<String, Prestart>>> =
    std::sync::LazyLock::new(Default::default);

struct PrestartSlot(String);

impl PrestartSlot {
    fn take(block_id: &str) -> Option<Self> {
        let mut map = PRESTARTING.lock().unwrap_or_else(|e| e.into_inner());
        match map.entry(block_id.to_string()) {
            std::collections::hash_map::Entry::Occupied(_) => None,
            std::collections::hash_map::Entry::Vacant(v) => {
                v.insert(Prestart::Preparing);
                Some(Self(block_id.to_string()))
            }
        }
    }

    /// Whether the pane closed while this was preparing.
    fn cancelled(&self) -> bool {
        matches!(PRESTARTING.lock().unwrap_or_else(|e| e.into_inner()).get(&self.0), Some(Prestart::Cancelled))
            || crate::backend::blockcontroller::is_closing(&self.0)
    }

    /// The task exists. If the pane closed since `cancelled` was asked, it ends here.
    fn running(&self, task: tokio::task::AbortHandle) {
        let mut map = PRESTARTING.lock().unwrap_or_else(|e| e.into_inner());
        match map.get_mut(&self.0) {
            Some(state @ Prestart::Preparing) => *state = Prestart::Running(task),
            _ => task.abort(),
        }
    }
}

/// End the pre-start of a pane that is closing, so a start still pulling its image can't create and run a
/// container for a pane that is gone. The caller stops the container afterwards. Nothing under way: nothing done.
pub(crate) fn cancel_prestart(block_id: &str) {
    // Aborted outside the lock: ending the task drops its slot, which takes the lock.
    let task = match PRESTARTING.lock().unwrap_or_else(|e| e.into_inner()).get_mut(block_id) {
        Some(state @ Prestart::Preparing) => {
            *state = Prestart::Cancelled;
            None
        }
        Some(Prestart::Running(task)) => Some(task.clone()),
        _ => None,
    };
    if let Some(task) = task {
        task.abort();
    }
}

impl Drop for PrestartSlot {
    fn drop(&mut self) {
        PRESTARTING.lock().unwrap_or_else(|e| e.into_inner()).remove(&self.0);
    }
}

/// Begin a new container agent's start (image, container, CLI) in the background, so its first message
/// finds a warm container. Only for a container agent whose pane has run nothing, and only once its account
/// resolves (the mounts carry the account's credentials: starting without them would make the wrong
/// container). The first message's own start-up shares the container's locks, so nothing is done twice.
pub(crate) async fn prestart(deps: &super::input::AgentTurnDeps, block_id: String) -> Result<(), String> {
    let Some(block) = deps.mstore.get::<crate::backend::obj::Block>(&block_id).map_err(|e| format!("prestart: load block: {e}"))? else {
        return Ok(());
    };
    let agent_mode = crate::backend::obj::meta_get_string(&block.meta, "agentMode", "host");
    let status = crate::backend::blockcontroller::get_controller(&block_id).map(|c| c.get_runtime_status().shellprocstatus);
    if !prestart_applies(&agent_mode, status.as_deref().unwrap_or("")) {
        return Ok(());
    }
    let Some(slot) = PrestartSlot::take(&block_id) else { return Ok(()) };
    let began = Instant::now();
    let cm = match deps.container_manager.get_or_explain().await {
        Ok(cm) => cm,
        Err(e) => {
            // Nobody asked for this start, so no turn ends with the reason: the pane's start row says it.
            let provider = crate::backend::obj::meta_get_string(&block.meta, "agentProvider", "claude");
            let frame = cli_notice::container_start_frame(&uuid::Uuid::new_v4().to_string(), &provider, "", ContainerStartState::Failed, None, None, Some(&e));
            cli_notice::append_to_pane(&deps.broker, &deps.filestore_gate, &deps.mstore, &block_id, &frame);
            return Err(e);
        }
    };
    let working_dir = crate::backend::obj::meta_get_string(&block.meta, "cmd:cwd", "");
    let mut env_vars = match super::input::build_persistent_spawn_env(
        deps.mstore.clone(),
        deps.id_store.clone(),
        deps.identity_store.clone(),
        Some(deps.broker.clone()),
        &block.meta,
        &block_id,
        &deps.auth_key,
        crate::backend::blockcontroller::cmd_env_of(&block.meta),
    )
    .await
    {
        Ok(env) => env,
        Err(e) => {
            tracing::info!(block_id = %block_id, error = %e, "container pre-start skipped: no account to start it with yet");
            return Ok(());
        }
    };
    let startup = ContainerStartup::for_block(
        cm,
        &block_id,
        &block.meta,
        &mut env_vars,
        &working_dir,
        deps.broker.clone(),
        deps.filestore_gate.clone(),
        deps.mstore.clone(),
    );
    // The pane may have closed while the account and mounts were resolved; closing sees the entry from here on.
    if slot.cancelled() {
        return Ok(());
    }
    // The slot outlives the task (the waiter below drops it), so the handle is always set on a live entry.
    let task = tokio::spawn({
        let slot_block = block_id.clone();
        async move {
            // A failure is on the pane as the start row's Failed state; the first message tries again.
            match startup.run_quietly().await {
                Ok(()) => startup.note_waiting_for_message(began.elapsed().as_secs_f64()),
                Err(e) => tracing::info!(block_id = %slot_block, error = %e, "container pre-start did not finish"),
            }
        }
    });
    slot.running(task.abort_handle());
    tokio::spawn(async move {
        // Holds the slot until the pre-start ends, however it ends (finished, failed or aborted).
        let _ = task.await;
        drop(slot);
    });
    Ok(())
}

/// The `result` frame that ends a turn with an error and carries `message`.
pub(crate) fn error_result_frame(message: &str) -> Value {
    json!({
        "type": "result",
        "is_error": true,
        "subtype": "error_during_execution",
        "error": {"message": format!("[AgentMux] {message}")}
    })
}

/// Runs `f` when dropped unless [disarmed](Self::disarm): closes a pane row
/// whose work was cancelled (a stop drops the start-up future mid-way).
struct IfDropped<F: Fn()> {
    armed: AtomicBool,
    f: F,
}

impl<F: Fn()> IfDropped<F> {
    fn new(f: F) -> Self {
        Self { armed: AtomicBool::new(true), f }
    }

    fn disarm(&self) {
        self.armed.store(false, Ordering::Relaxed);
    }
}

impl<F: Fn()> Drop for IfDropped<F> {
    fn drop(&mut self) {
        if self.armed.load(Ordering::Relaxed) {
            (self.f)();
        }
    }
}

impl ContainerStartup {
    /// Start the container, then make sure the provider CLI is in it. The pane
    /// shows each step that takes time (frames persisted, so they survive a
    /// reload). On failure it gets a plain-language error frame and the same
    /// text is returned.
    pub(crate) async fn run(&self) -> Result<(), String> {
        self.run_reporting(true).await
    }

    /// [`run`](Self::run) for a start nobody asked for yet (the pre-start): a failure shows as its start or
    /// install row, with no error result on the pane, since no turn is there to end with one.
    pub(crate) async fn run_quietly(&self) -> Result<(), String> {
        // No turn holds the container here, so an agent whose container runs an older agent image than this
        // AgentMux moves to the current one before it starts, and the pane says so. (A turn's own start holds the
        // update gate shared for the whole turn, so it cannot do this: it would always find the gate taken.)
        crate::server::app_api::container_update::update_if_idle(
            &self.cm,
            &self.container_name,
            &self.image,
            &self.provider_id,
            &self.block_id,
            &self.broker,
            &self.filestore,
            &self.mstore,
        )
        .await;
        self.run_reporting(false).await
    }

    async fn run_reporting(&self, report_failure: bool) -> Result<(), String> {
        // The workspace is mounted into the container: signing keys an earlier launch wrote into its
        // .mcp.json come out before it starts (and a launch no longer writes them).
        if let Some(dir) = self.mount_spec.workspace_host_dir.clone() {
            let _ = tokio::task::spawn_blocking(move || crate::backend::mcp_json_identity::strip_workspace_signing_keys(&dir)).await;
        }
        // On macOS a Keychain-backed account has no credentials file to mount: its login is staged for the container.
        let mut mount_spec = self.mount_spec.clone();
        if mount_spec.claude_config_host_dir.is_none() {
            if let Some(config_dir) = mount_spec.keychain_config_dir.clone() {
                mount_spec.claude_config_host_dir = self.cm.stage_keychain_login(&self.container_name, &config_dir).await;
            }
        }
        let image = resolve_container_image(&self.image);
        let began = Instant::now();
        let start_id = uuid::Uuid::new_v4().to_string();
        // Whether anything was shown: a container that was already up says
        // nothing, so a normal turn gets no start-up row.
        let shown = AtomicBool::new(false);
        let start_row = |state: ContainerStartState, percent: Option<u8>, image: &str, error: Option<&str>| {
            let seconds = matches!(state, ContainerStartState::Ready).then(|| began.elapsed().as_secs_f64());
            let frame = cli_notice::container_start_frame(&start_id, &self.provider_id, image, state, percent, seconds, error);
            cli_notice::append_to_pane(&self.broker, &self.filestore, &self.mstore, &self.block_id, &frame);
        };
        // A stop drops this future while the row says "Downloading…"; the
        // guard then ends that row so a reload doesn't show it for good.
        let row_open = IfDropped::new(|| {
            if shown.load(Ordering::Relaxed) {
                start_row(ContainerStartState::Failed, None, &image, Some("Stopped before the container was ready"));
            }
        });
        let observe = |event: StartEvent<'_>| {
            shown.store(true, Ordering::Relaxed);
            match event {
                StartEvent::Pulling { image, progress } => {
                    start_row(ContainerStartState::Pulling, progress.map(|p| p.percent()), image, None)
                }
                StartEvent::Starting => start_row(ContainerStartState::Starting, None, &image, None),
            }
        };
        if let Err(e) = self
            .cm
            .ensure_running_observed(&self.container_name, &image, &self.volumes, &[], &mount_spec, &observe)
            .await
        {
            if shown.load(Ordering::Relaxed) {
                start_row(ContainerStartState::Failed, None, &image, Some(&e.user_message()));
            }
            row_open.disarm();
            return Err(self.fail(&e, report_failure));
        }
        // Dev servers the agent asked to have kept up come back with the container.
        crate::backend::dev_keepalive::restore_in_background(&self.cm, &self.container_name);
        if shown.load(Ordering::Relaxed) {
            start_row(ContainerStartState::Ready, None, &image, None);
        } else {
            let (cm, name) = (&self.cm, &self.container_name);
            crate::server::app_api::container_update::note_outdated(cm, name, &self.provider_id, &self.block_id, &self.broker, &self.filestore, &self.mstore).await;
        }
        row_open.disarm();

        let Some(install) = CliInstall::for_provider(&self.provider_id, &self.container_command) else {
            return Ok(());
        };
        let install_id = uuid::Uuid::new_v4().to_string();
        let notice = |state: InstallState, seconds: Option<f64>, error: Option<&str>| {
            let frame = cli_notice::install_frame(&install_id, &install.provider_id, &install.version, state, seconds, error);
            cli_notice::append_to_pane(&self.broker, &self.filestore, &self.mstore, &self.block_id, &frame);
        };

        let started_install = AtomicBool::new(false);
        // Same for a stop during the install, which also reaps it in the container.
        let install_open = IfDropped::new(|| {
            if started_install.load(Ordering::Relaxed) {
                notice(InstallState::Failed, None, Some("Stopped before the install finished"));
            }
        });
        let outcome = self
            .cm
            .provision_cli(&self.container_name, &install, || {
                started_install.store(true, Ordering::Relaxed);
                notice(InstallState::Installing, None, None);
            })
            .await;
        install_open.disarm();
        match outcome {
            Ok(CliProvision::Present) => Ok(()),
            Ok(CliProvision::Installed { seconds }) => {
                notice(InstallState::Installed, Some(seconds), None);
                Ok(())
            }
            Err(e) => {
                if started_install.load(Ordering::Relaxed) {
                    let detail = match &e {
                        ContainerError::CliInstall { detail, .. } => detail.clone(),
                        other => other.to_string(),
                    };
                    notice(InstallState::Failed, None, Some(&detail));
                }
                Err(self.fail(&e, report_failure))
            }
        }
    }

    /// A pre-start of a fresh pane that finished with no message on its way: the pane would sit silent, so say it is ready
    /// and waiting for the user. Nothing is said when a turn has begun (its own rows speak) or the pane has closed.
    fn note_waiting_for_message(&self, seconds: f64) {
        let status = crate::backend::blockcontroller::get_controller(&self.block_id).map(|c| c.get_runtime_status().shellprocstatus);
        if !prestart_applies("container", status.as_deref().unwrap_or("")) {
            return;
        }
        // A resumed pane has its history and composer on screen, and is not "starting": it gets no row at each reopen.
        let resumed = match self.mstore.get::<crate::backend::obj::Block>(&self.block_id) {
            Ok(Some(block)) => !crate::backend::obj::meta_get_string(&block.meta, "agent:sessionid", "").is_empty(),
            _ => true,
        };
        if resumed {
            return;
        }
        let frame = cli_notice::agent_ready_frame(&uuid::Uuid::new_v4().to_string(), &self.provider_id, seconds);
        cli_notice::append_to_pane(&self.broker, &self.filestore, &self.mstore, &self.block_id, &frame);
    }

    /// Log `e`, write its plain-language message to the pane, and return it.
    fn fail(&self, e: &ContainerError, report: bool) -> String {
        tracing::warn!(block_id = %self.block_id, container = %self.container_name, error = %e, "container agent could not start");
        let message = e.user_message();
        if !report {
            return message;
        }
        let line = format!("{}\n", error_result_frame(&message));
        let zone = crate::backend::transcript::append::resolve_global_output_zone(
            &Some(self.mstore.clone()),
            &self.block_id,
        );
        crate::backend::blockcontroller::shell::handle_append_block_file(
            &self.broker,
            &self.block_id,
            crate::backend::blockcontroller::subprocess::SUBPROCESS_OUTPUT_SUBJECT,
            line.as_bytes(),
            Some(&self.filestore),
            zone.as_deref(),
        );
        message
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_prestart_applies_only_to_a_container_pane_that_has_run_nothing() {
        use crate::backend::blockcontroller::{STATUS_DONE, STATUS_INIT, STATUS_RUNNING};
        assert!(prestart_applies("container", STATUS_INIT));
        assert!(!prestart_applies("container", STATUS_RUNNING), "a turn is running: its own start-up has it");
        assert!(!prestart_applies("container", STATUS_DONE), "a turn has run");
        assert!(!prestart_applies("container", ""), "no controller yet");
        assert!(!prestart_applies("host", STATUS_INIT));
    }

    #[test]
    fn a_second_prestart_for_the_same_block_does_nothing_until_the_first_ends() {
        let first = PrestartSlot::take("block-prestart-slot").expect("the first takes it");
        assert!(PrestartSlot::take("block-prestart-slot").is_none());
        assert!(PrestartSlot::take("block-prestart-other").is_some(), "another block is its own");
        drop(first);
        assert!(PrestartSlot::take("block-prestart-slot").is_some(), "free again once it ended");
    }

    #[tokio::test]
    async fn closing_a_pane_ends_its_prestart_and_frees_its_slot() {
        let slot = PrestartSlot::take("block-prestart-cancel").expect("the first takes it");
        assert!(!slot.cancelled());
        let task = tokio::spawn(std::future::pending::<()>());
        slot.running(task.abort_handle());
        let waiter = tokio::spawn(async move {
            let _ = task.await;
            drop(slot);
        });
        cancel_prestart("block-prestart-cancel");
        waiter.await.unwrap();
        assert!(PrestartSlot::take("block-prestart-cancel").is_some(), "free once it ended");
        // Nothing under way, or no such block: nothing happens.
        cancel_prestart("block-prestart-cancel");
        cancel_prestart("block-never-prestarted");
    }

    #[tokio::test]
    async fn a_pane_closed_while_the_prestart_prepares_never_starts_it() {
        let slot = PrestartSlot::take("block-prestart-early").expect("taken");
        cancel_prestart("block-prestart-early");
        assert!(slot.cancelled(), "the prestart sees the close before it spawns");
        // Closed in the instant between that check and the spawn: the task ends as soon as it is registered.
        let task = tokio::spawn(std::future::pending::<()>());
        slot.running(task.abort_handle());
        assert!(task.await.unwrap_err().is_cancelled());
        drop(slot);
        assert!(PrestartSlot::take("block-prestart-early").is_some(), "free again");
    }

    #[test]
    fn the_spec_comes_from_the_panes_meta_and_the_env_the_turn_would_use() {
        let mut meta = crate::backend::obj::MetaMapType::new();
        meta.insert("agentId".into(), json!("3f2a-agent"));
        meta.insert("agent:container_image".into(), json!("example/agent:1"));
        meta.insert("agent:container_volumes".into(), json!(r#"["/a:/b"]"#));
        meta.insert("agentProvider".into(), json!("codex"));
        meta.insert("agent:container_command".into(), json!("codex"));
        let mut env = std::collections::HashMap::new();
        let spec = ContainerSpec::of(&meta, &mut env, "/work");
        assert_eq!(spec.container_name, crate::backend::container::container_name_for_slug("3f2a-agent"));
        assert_eq!((spec.image.as_str(), spec.provider_id.as_str(), spec.container_command.as_str()), ("example/agent:1", "codex", "codex"));
        assert_eq!(spec.volumes, vec!["/a:/b".to_string()]);
        // Nothing set: the defaults the turn uses.
        let bare = ContainerSpec::of(&crate::backend::obj::MetaMapType::new(), &mut env, "");
        assert_eq!((bare.provider_id.as_str(), bare.container_command.as_str(), bare.image.as_str()), ("claude", "claude", ""));
    }

    #[test]
    fn the_error_frame_is_a_terminal_error_result_with_the_agentmux_prefix() {
        let frame = error_result_frame("Couldn't start the container for this agent: x");
        assert_eq!(frame["type"], "result");
        assert_eq!(frame["is_error"], true);
        assert_eq!(
            frame["error"]["message"],
            "[AgentMux] Couldn't start the container for this agent: x"
        );
    }
}
