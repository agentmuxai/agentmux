// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Getting a container agent's container ready for a turn: the container is
//! running, and the provider CLI is installed in it.
//!
//! Both places that start a container turn (`agentinput` and `agent.send`)
//! go through [`prepare`], so a failure reads the same in the pane whichever
//! way the message arrived.
//!
//! Spec: docs/specs/SPEC_CONTAINER_AGENTS_WORK_FOR_EVERYONE_2026_10_07.md.

use std::sync::Arc;

use serde_json::{json, Value};

use crate::backend::cli_notice::{self, InstallState};
use crate::backend::container::{CliProvision, ContainerError, ContainerManager, ContainerMountSpec};
use crate::backend::container_cli::CliInstall;
use crate::backend::container_image::resolve_container_image;

/// Everything [`prepare`] needs from the turn that is starting.
pub(crate) struct ContainerTurnPrep<'a> {
    pub cm: &'a ContainerManager,
    pub block_id: &'a str,
    pub container_name: &'a str,
    /// The stored `agent:container_image`; empty means the default image.
    pub image: &'a str,
    pub volumes: &'a [String],
    pub mount_spec: &'a ContainerMountSpec,
    pub provider_id: &'a str,
    pub container_command: &'a str,
    pub broker: &'a Arc<crate::backend::mps::Broker>,
    pub filestore: &'a Arc<crate::backend::storage::filestore::FileStore>,
    pub mstore: &'a Arc<crate::backend::storage::store::Store>,
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

/// Start the container, then make sure the provider CLI is in it. On failure
/// the pane gets a plain-language error frame (persisted, so it survives a
/// reload) and the same text is returned.
pub(crate) async fn prepare(p: ContainerTurnPrep<'_>) -> Result<(), String> {
    let image = resolve_container_image(p.image);
    if let Err(e) = p
        .cm
        .ensure_running(p.container_name, &image, p.volumes, &[], p.mount_spec)
        .await
    {
        return Err(fail(&p, &e));
    }

    let Some(install) = CliInstall::for_provider(p.provider_id, p.container_command) else {
        return Ok(());
    };
    let install_id = uuid::Uuid::new_v4().to_string();
    let notice = |state: InstallState, seconds: Option<f64>, error: Option<&str>| {
        let frame = cli_notice::install_frame(&install_id, &install.provider_id, &install.version, state, seconds, error);
        cli_notice::append_to_pane(p.broker, p.filestore, p.mstore, p.block_id, &frame);
    };

    let mut started_install = false;
    let outcome = p
        .cm
        .provision_cli(p.container_name, &install, || {
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
            Err(fail(&p, &e))
        }
    }
}

/// Log `e`, write its plain-language message to the pane, and return it.
fn fail(p: &ContainerTurnPrep<'_>, e: &ContainerError) -> String {
    tracing::warn!(block_id = %p.block_id, container = %p.container_name, error = %e, "container agent could not start");
    let message = e.user_message();
    let line = format!("{}\n", error_result_frame(&message));
    let zone = crate::backend::blockcontroller::shell::resolve_global_output_zone(
        &Some(p.mstore.clone()),
        p.block_id,
    );
    crate::backend::blockcontroller::shell::handle_append_block_file(
        p.broker,
        p.block_id,
        crate::backend::blockcontroller::subprocess::SUBPROCESS_OUTPUT_SUBJECT,
        line.as_bytes(),
        Some(p.filestore),
        zone.as_deref(),
    );
    message
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
