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

/// The `result` frame that ends a turn with an error and carries `message`.
pub(crate) fn error_result_frame(message: &str) -> Value {
    json!({
        "type": "result",
        "is_error": true,
        "subtype": "error_during_execution",
        "error": {"message": format!("[AgentMux] {message}")}
    })
}

/// Ends the start-up row if the start-up is dropped before it did (a stop).
struct UnfinishedRow<'a> {
    done: AtomicBool,
    shown: &'a AtomicBool,
    write: &'a (dyn Fn(ContainerStartState, Option<u8>, &str, Option<&str>) + Sync),
    image: &'a str,
}

impl Drop for UnfinishedRow<'_> {
    fn drop(&mut self) {
        if !self.done.load(Ordering::Relaxed) && self.shown.load(Ordering::Relaxed) {
            (self.write)(ContainerStartState::Failed, None, self.image, Some("Stopped before the container was ready"));
        }
    }
}

impl ContainerStartup {
    /// Start the container, then make sure the provider CLI is in it. The pane
    /// shows each step that takes time (frames persisted, so they survive a
    /// reload). On failure it gets a plain-language error frame and the same
    /// text is returned.
    pub(crate) async fn run(&self) -> Result<(), String> {
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
        let row_open = UnfinishedRow { done: AtomicBool::new(false), shown: &shown, write: &start_row, image: &image };
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
            .ensure_running_observed(&self.container_name, &image, &self.volumes, &[], &self.mount_spec, &observe)
            .await
        {
            if shown.load(Ordering::Relaxed) {
                start_row(ContainerStartState::Failed, None, &image, Some(&e.user_message()));
            }
            row_open.done.store(true, Ordering::Relaxed);
            return Err(self.fail(&e));
        }
        if shown.load(Ordering::Relaxed) {
            start_row(ContainerStartState::Ready, None, &image, None);
        }
        row_open.done.store(true, Ordering::Relaxed);

        let Some(install) = CliInstall::for_provider(&self.provider_id, &self.container_command) else {
            return Ok(());
        };
        let install_id = uuid::Uuid::new_v4().to_string();
        let notice = |state: InstallState, seconds: Option<f64>, error: Option<&str>| {
            let frame = cli_notice::install_frame(&install_id, &install.provider_id, &install.version, state, seconds, error);
            cli_notice::append_to_pane(&self.broker, &self.filestore, &self.mstore, &self.block_id, &frame);
        };

        let mut started_install = false;
        let outcome = self
            .cm
            .provision_cli(&self.container_name, &install, || {
                started_install = true;
                notice(InstallState::Installing, None, None);
            })
            .await;
        match outcome {
            Ok(CliProvision::Present) => Ok(()),
            Ok(CliProvision::Installed { seconds }) => {
                notice(InstallState::Installed, Some(seconds), None);
                Ok(())
            }
            Err(e) => {
                if started_install {
                    let detail = match &e {
                        ContainerError::CliInstall { detail, .. } => detail.clone(),
                        other => other.to_string(),
                    };
                    notice(InstallState::Failed, None, Some(&detail));
                }
                Err(self.fail(&e))
            }
        }
    }

    /// Log `e`, write its plain-language message to the pane, and return it.
    fn fail(&self, e: &ContainerError) -> String {
        tracing::warn!(block_id = %self.block_id, container = %self.container_name, error = %e, "container agent could not start");
        let message = e.user_message();
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
