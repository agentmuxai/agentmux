// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Where a new agent runs: the `runtime`, `container_image`, `memory_mb` and `cpus` of `Agents spawn` (a worker) and
//! `Agents create` (a named agent from a template).
//!
//! Without them a worker runs as its leader does (`copy_definition` keeps the type and the image), and a created agent
//! as its template says. With them, the new agent can run in a container when its caller runs on the host. The
//! protections are the ones the rest of the container API has: a container caller starts only container agents
//! (a container is not a way to a host process) and does not set limits (it asks a host agent, as for its own), and
//! a container is refused when Docker is not there, with the reason.

use agentmux_common::api_types::ContainerAsk;

use super::*;

/// Where the new agent runs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) enum Placement {
    /// Nothing was asked: what its source (the leader or the template) gives.
    #[default]
    AsSource,
    /// On the host.
    Native,
    Container(ContainerPlan),
}

/// A container agent as asked for: the image, and the limits to store (`None`: leave them automatic).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ContainerPlan {
    pub image: String,
    pub memory_mb: Option<String>,
    pub cpus: Option<String>,
}

impl Placement {
    /// How the new agent is described to the user in the approval prompt; empty for `AsSource`.
    pub(crate) fn describe(&self) -> String {
        match self {
            Placement::AsSource => String::new(),
            Placement::Native => " on this computer".to_string(),
            Placement::Container(c) => {
                let mut s = format!(" in a container ({})", c.image);
                if let Some(m) = &c.memory_mb {
                    s.push_str(&format!(", memory {}", if m == "0" { "unlimited".to_string() } else if m.is_empty() { "auto".to_string() } else { format!("{m} MB") }));
                }
                if let Some(c) = &c.cpus {
                    s.push_str(&format!(", CPUs {}", if c == "0" { "unlimited".to_string() } else if c.is_empty() { "auto".to_string() } else { c.clone() }));
                }
                s
            }
        }
    }
}

/// Check what `ask` wants, without making anything. `caller_in_container`: the asking agent runs in a container.
/// `source_in_container` and `source_image`: how the source (the leader, or the template) runs, which `runtime` and
/// `container_image` default to. `docker`: whether a container can be made here, or why not.
pub(crate) fn decide(
    ask: &ContainerAsk,
    caller_in_container: bool,
    source_in_container: bool,
    source_image: &str,
    docker: Result<(), String>,
) -> Result<Placement, SpawnError> {
    use crate::backend::container::limits::normalize;
    // A container caller making an agent from a host template gets no say: a host agent is a way out of the container.
    if ask.is_empty() && !(caller_in_container && !source_in_container) {
        return Ok(Placement::AsSource);
    }
    let runtime = match ask.runtime.as_deref().map(str::trim).filter(|r| !r.is_empty()) {
        None => None,
        Some("native" | "host") => Some(false),
        Some("container") => Some(true),
        Some(other) => {
            return Err(refuse(StatusCode::BAD_REQUEST, format!("INVALID_ARG: runtime {other:?}: use native or container")));
        }
    };
    let wants_limits = ask.memory_mb.is_some() || ask.cpus.is_some();
    let in_container = runtime.unwrap_or(source_in_container);
    if !in_container {
        if ask.container_image.is_some() || wants_limits {
            return Err(refuse(
                StatusCode::BAD_REQUEST,
                "INVALID_ARG: container_image, memory_mb and cpus are for a container agent: add runtime: container",
            ));
        }
        if caller_in_container {
            return Err(refuse(
                StatusCode::FORBIDDEN,
                "CONTAINER_AGENT: a container agent starts only container agents, never one on the host. Ask a host agent, \
                 your steward or the operator",
            ));
        }
        return Ok(Placement::Native);
    }
    if caller_in_container && wants_limits {
        return Err(refuse(
            StatusCode::FORBIDDEN,
            "CONTAINER_AGENT: a container agent cannot set a container's limits. Leave memory_mb and cpus out, or ask a host \
             agent, your steward or the operator, saying how much the work needs and why",
        ));
    }
    let image = crate::server::agent_handlers::chosen_container_image(ask.container_image.as_deref(), "container", source_image)
        .map_err(|e| refuse(StatusCode::BAD_REQUEST, format!("INVALID_ARG: {e}")))?;
    let image = if image.trim().is_empty() { crate::backend::container_image::DEFAULT_AGENT_IMAGE.to_string() } else { image };
    let memory_mb = ask.memory_mb.as_ref().map(|v| normalize(v, "memory_mb")).transpose().map_err(|e| refuse(StatusCode::BAD_REQUEST, format!("INVALID_ARG: {e}")))?;
    let cpus = ask.cpus.as_ref().map(|v| normalize(v, "cpus")).transpose().map_err(|e| refuse(StatusCode::BAD_REQUEST, format!("INVALID_ARG: {e}")))?;
    docker.map_err(|why| refuse(StatusCode::SERVICE_UNAVAILABLE, format!("DOCKER_UNAVAILABLE: a container agent needs Docker, and {why}. Use runtime: native, or ask the user to start Docker")))?;
    Ok(Placement::Container(ContainerPlan { image, memory_mb, cpus }))
}

/// [`decide`] for a caller `caller` and a source defined by `source`, asking this machine whether Docker is there.
pub(crate) async fn decide_here(
    state: &AppState,
    ask: &ContainerAsk,
    caller: &AgentDefinition,
    source: &AgentDefinition,
) -> Result<Placement, SpawnError> {
    // Only a request for a container needs the answer; a plain one never starts a Docker lookup.
    let wants_container = ask.runtime.as_deref().map(str::trim) == Some("container") || (ask.runtime.is_none() && source.agent_type == "container");
    let docker = if wants_container && !ask.is_empty() {
        state.container_manager.get_or_explain().await.map(|_| ())
    } else {
        Ok(())
    };
    decide(ask, caller.agent_type == "container", source.agent_type == "container", &source.container_image, docker)
}

/// Make `def` (just inserted) a container agent as `plan` says, and store its limits. A native placement on a
/// definition that was a container one (a template's) is `agent_type` and `environment` back to the host's.
pub(crate) fn apply(state: &AppState, def: &mut AgentDefinition, placement: &Placement) -> Result<(), String> {
    match placement {
        Placement::AsSource => return Ok(()),
        Placement::Native => {
            def.agent_type = "host".to_string();
            def.environment = "local".to_string();
            def.container_image = String::new();
        }
        Placement::Container(plan) => {
            def.agent_type = "container".to_string();
            def.environment = "docker".to_string();
            def.container_image = plan.image.clone();
            if plan.memory_mb.is_some() || plan.cpus.is_some() {
                let (old_memory, old_cpus) = state.mstore.container_limits_get(&def.id).ok().flatten().unwrap_or_default();
                state
                    .mstore
                    .container_limits_set(
                        &def.id,
                        plan.memory_mb.as_deref().unwrap_or(&old_memory),
                        plan.cpus.as_deref().unwrap_or(&old_cpus),
                    )
                    .map_err(|e| format!("store its container limits: {e}"))?;
            }
        }
    }
    state.mstore.agent_def_update(def).map_err(|e| format!("set where it runs: {e}"))?;
    Ok(())
}

#[cfg(test)]
#[path = "../tests/agent_spawn_container_tests.rs"]
mod tests;
